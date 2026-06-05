//! Unified descriptor dispatch: [`AnyDescriptor`] + [`parse_loop`].
//!
//! [`AnyDescriptor`] is generated from a single declarative list
//! (`declare_descriptors!`) — one line per crate-implemented descriptor tag.
//! The list is the single source of truth: it produces the enum, the
//! `From<T>` conversions, and the tag → type dispatcher, and a drift test
//! pins each tag literal to the type's [`crate::traits::DescriptorDef::TAG`].
//!
//! [`parse_loop`] lazily walks a raw descriptor loop (the variable-length
//! `descriptor()` sequence inside tables), yielding one [`AnyDescriptor`] per
//! entry. It never panics: a malformed entry whose length is known yields an
//! `Err` and iteration continues; a truncated final header/body yields one
//! final `Err` and then fuses.
//!
//! ```
//! use dvb_si::descriptors::{parse_loop, AnyDescriptor};
//!
//! // A two-descriptor loop: short_event (tag 0x4D, "eng" / "News") then an
//! // unrecognised private tag 0xA7 — the walker yields a typed value for the
//! // first and `Unknown` for the second, never panicking.
//! let loop_bytes = [
//!     0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
//!     0xA7, 0x02, 0xCA, 0xFE,                               // unknown 0xA7
//! ];
//! let items: Vec<_> = parse_loop(&loop_bytes).collect();
//! assert_eq!(items.len(), 2);
//! match items[0].as_ref().unwrap() {
//!     AnyDescriptor::ShortEvent(se) => {
//!         assert_eq!(se.language_code.as_str(), "eng");
//!         assert_eq!(se.event_name.decode(), "Hi");
//!     }
//!     other => panic!("expected ShortEvent, got {other:?}"),
//! }
//! assert!(matches!(items[1].as_ref().unwrap(), AnyDescriptor::Unknown { tag: 0xA7, .. }));
//! ```
//!
//! # Adding a descriptor
//!
//! 1. Create the module with the wire layout, a `pub const TAG: u8`, and the
//!    symmetric [`dvb_common::Parse`]/[`dvb_common::Serialize`] impls +
//!    round-trip tests (copy an existing module).
//! 2. `impl DescriptorDef` for the type (`TAG` from the module const, `NAME`
//!    in SCREAMING_SNAKE without the `_descriptor` suffix).
//! 3. Add one line to the `declare_descriptors!` invocation below — the enum
//!    variant, dispatcher arm, and drift test are generated from it.
//! 4. The integration completeness test walks the generated
//!    [`AnyDescriptor::DISPATCHED_TAGS`] automatically — no test edits needed.

/// Declares [`AnyDescriptor`] + its dispatcher from one tag list.
///
/// Each line is `Variant = 0xTAG => module::Type[<'a>]`. The optional trailing
/// `@no_dispatch …` section adds variants that are NOT reachable from the
/// generated dispatcher (private / context-dependent tags such as 0x83
/// logical_channel) — the variant exists for callers that opt in via the
/// registry, but `dispatch` never produces it.
macro_rules! declare_descriptors {
    (
        $lt:lifetime;
        $( $variant:ident = $tag:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
        $( ; @no_dispatch $( $nd_variant:ident => $($nd_path:ident)::+ $(<$nd_plt:lifetime>)? ),+ $(,)? )?
    ) => {
        /// Every crate-implemented descriptor, plus an `Unknown` fallthrough.
        ///
        /// serde uses external tagging with camelCase variant keys —
        /// a parsed short_event_descriptor serializes as `{"shortEvent": {…}}`.
        /// Variant names map 1:1 to the descriptor modules; see each module
        /// for the wire layout.
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnyDescriptor<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            $($(
                #[allow(missing_docs)]
                $nd_variant($($nd_path)::+ $(<$nd_plt>)?),
            )+)?
            /// Runtime-registered custom descriptor (see [`DescriptorRegistry`]).
            ///
            /// [`DescriptorRegistry`]: crate::descriptors::registry::DescriptorRegistry
            Other {
                /// The raw descriptor_tag byte.
                tag: u8,
                /// The parsed, type-erased descriptor value. Use
                /// [`DescriptorObject::as_any`][crate::descriptors::registry::DescriptorObject::as_any]
                /// followed by `downcast_ref` to recover the concrete type.
                #[cfg_attr(
                    feature = "serde",
                    serde(serialize_with = "crate::descriptors::registry::serialize_erased")
                )]
                value: Box<dyn crate::descriptors::registry::DescriptorObject>,
            },
            /// Tag with no typed implementation; `body` is the payload sans
            /// the 2-byte (tag, length) header.
            Unknown {
                /// The raw descriptor_tag byte.
                tag: u8,
                /// The raw payload bytes (descriptor_length bytes).
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyDescriptor<$lt> {
                fn from(d: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(d)
                }
            }
        )+
        $($(
            impl<$lt> From<$($nd_path)::+ $(<$nd_plt>)?> for AnyDescriptor<$lt> {
                fn from(d: $($nd_path)::+ $(<$nd_plt>)?) -> Self {
                    Self::$nd_variant(d)
                }
            }
        )+)?

        impl<$lt> AnyDescriptor<$lt> {
            /// Every tag the generated dispatcher routes (excludes `@no_dispatch`
            /// variants and [`AnyDescriptor::Unknown`]).
            pub const DISPATCHED_TAGS: &'static [u8] = &[$($tag),+];

            /// Diagnostic name of the contained descriptor — the type's
            /// [`DescriptorDef::NAME`](crate::traits::DescriptorDef::NAME)
            /// (`"SHORT_EVENT"`, `"NETWORK_NAME"`, …); `"CUSTOM"` for
            /// [`AnyDescriptor::Other`] (runtime-registered) and `"UNKNOWN"`
            /// for [`AnyDescriptor::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::DescriptorDef>::NAME,
                    )+
                    $($(
                        Self::$nd_variant(_) =>
                            <$($nd_path)::+ as crate::traits::DescriptorDef>::NAME,
                    )+)?
                    Self::Other { .. } => "CUSTOM",
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Parse one full descriptor (2-byte header included) by its tag.
            ///
            /// `None` means no typed implementation exists for `tag` (the
            /// caller turns that into [`AnyDescriptor::Unknown`]). `Some(Err)`
            /// is a typed parse failure for a recognised tag.
            pub(crate) fn dispatch(tag: u8, full: &$lt [u8]) -> Option<crate::Result<Self>> {
                use dvb_common::Parse;
                match tag {
                    $(
                        $tag => Some(<$($path)::+>::parse(full).map(Self::$variant)),
                    )+
                    _ => None,
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn tag_literals_match_descriptor_def() {
                use crate::traits::DescriptorDef;
                $(
                    assert_eq!(
                        $tag,
                        <$($path)::+ as DescriptorDef>::TAG,
                        concat!("tag literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as DescriptorDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
                $($(
                    assert!(
                        !<$($nd_path)::+ as DescriptorDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($nd_variant)),
                    );
                )+)?
            }
        }
    };
}

declare_descriptors! {'a;
    // MPEG-2 systems descriptors (ISO/IEC 13818-1) used outside table context.
    Registration = 0x05 => crate::descriptors::registration::RegistrationDescriptor<'a>,
    DataStreamAlignment = 0x06 => crate::descriptors::data_stream_alignment::DataStreamAlignmentDescriptor,
    Ca = 0x09 => crate::descriptors::ca::CaDescriptor<'a>,
    Iso639Language = 0x0A => crate::descriptors::iso_639_language::Iso639LanguageDescriptor,
    PrivateDataIndicator = 0x0F => crate::descriptors::private_data_indicator::PrivateDataIndicatorDescriptor,
    // DVB descriptors (ETSI EN 300 468) — contiguous 0x40..=0x7F.
    NetworkName = 0x40 => crate::descriptors::network_name::NetworkNameDescriptor<'a>,
    ServiceList = 0x41 => crate::descriptors::service_list::ServiceListDescriptor,
    Stuffing = 0x42 => crate::descriptors::stuffing::StuffingDescriptor<'a>,
    SatelliteDeliverySystem = 0x43 => crate::descriptors::satellite_delivery_system::SatelliteDeliverySystemDescriptor,
    CableDeliverySystem = 0x44 => crate::descriptors::cable_delivery_system::CableDeliverySystemDescriptor,
    VbiData = 0x45 => crate::descriptors::vbi_data::VbiDataDescriptor<'a>,
    VbiTeletext = 0x46 => crate::descriptors::vbi_teletext::VbiTeletextDescriptor,
    BouquetName = 0x47 => crate::descriptors::bouquet_name::BouquetNameDescriptor<'a>,
    Service = 0x48 => crate::descriptors::service::ServiceDescriptor<'a>,
    CountryAvailability = 0x49 => crate::descriptors::country_availability::CountryAvailabilityDescriptor,
    Linkage = 0x4A => crate::descriptors::linkage::LinkageDescriptor<'a>,
    NvodReference = 0x4B => crate::descriptors::nvod_reference::NvodReferenceDescriptor,
    TimeShiftedService = 0x4C => crate::descriptors::time_shifted_service::TimeShiftedServiceDescriptor,
    ShortEvent = 0x4D => crate::descriptors::short_event::ShortEventDescriptor<'a>,
    ExtendedEvent = 0x4E => crate::descriptors::extended_event::ExtendedEventDescriptor<'a>,
    TimeShiftedEvent = 0x4F => crate::descriptors::time_shifted_event::TimeShiftedEventDescriptor,
    Component = 0x50 => crate::descriptors::component::ComponentDescriptor<'a>,
    Mosaic = 0x51 => crate::descriptors::mosaic::MosaicDescriptor,
    StreamIdentifier = 0x52 => crate::descriptors::stream_identifier::StreamIdentifierDescriptor,
    CaIdentifier = 0x53 => crate::descriptors::ca_identifier::CaIdentifierDescriptor,
    Content = 0x54 => crate::descriptors::content::ContentDescriptor,
    ParentalRating = 0x55 => crate::descriptors::parental_rating::ParentalRatingDescriptor,
    Teletext = 0x56 => crate::descriptors::teletext::TeletextDescriptor,
    Telephone = 0x57 => crate::descriptors::telephone::TelephoneDescriptor<'a>,
    LocalTimeOffset = 0x58 => crate::descriptors::local_time_offset::LocalTimeOffsetDescriptor,
    Subtitling = 0x59 => crate::descriptors::subtitling::SubtitlingDescriptor,
    TerrestrialDeliverySystem = 0x5A => crate::descriptors::terrestrial_delivery_system::TerrestrialDeliverySystemDescriptor,
    MultilingualNetworkName = 0x5B => crate::descriptors::multilingual_network_name::MultilingualNetworkNameDescriptor<'a>,
    MultilingualBouquetName = 0x5C => crate::descriptors::multilingual_bouquet_name::MultilingualBouquetNameDescriptor<'a>,
    MultilingualServiceName = 0x5D => crate::descriptors::multilingual_service_name::MultilingualServiceNameDescriptor<'a>,
    MultilingualComponent = 0x5E => crate::descriptors::multilingual_component::MultilingualComponentDescriptor<'a>,
    PrivateDataSpecifier = 0x5F => crate::descriptors::private_data_specifier::PrivateDataSpecifierDescriptor,
    ServiceMove = 0x60 => crate::descriptors::service_move::ServiceMoveDescriptor,
    ShortSmoothingBuffer = 0x61 => crate::descriptors::short_smoothing_buffer::ShortSmoothingBufferDescriptor<'a>,
    FrequencyList = 0x62 => crate::descriptors::frequency_list::FrequencyListDescriptor,
    PartialTransportStream = 0x63 => crate::descriptors::partial_transport_stream::PartialTransportStreamDescriptor,
    DataBroadcast = 0x64 => crate::descriptors::data_broadcast::DataBroadcastDescriptor<'a>,
    Scrambling = 0x65 => crate::descriptors::scrambling::ScramblingDescriptor,
    DataBroadcastId = 0x66 => crate::descriptors::data_broadcast_id::DataBroadcastIdDescriptor<'a>,
    TransportStream = 0x67 => crate::descriptors::transport_stream::TransportStreamDescriptor<'a>,
    Dsng = 0x68 => crate::descriptors::dsng::DsngDescriptor<'a>,
    Pdc = 0x69 => crate::descriptors::pdc::PdcDescriptor,
    Ac3 = 0x6A => crate::descriptors::ac3::Ac3Descriptor<'a>,
    AncillaryData = 0x6B => crate::descriptors::ancillary_data::AncillaryDataDescriptor,
    CellList = 0x6C => crate::descriptors::cell_list::CellListDescriptor,
    CellFrequencyLink = 0x6D => crate::descriptors::cell_frequency_link::CellFrequencyLinkDescriptor,
    AnnouncementSupport = 0x6E => crate::descriptors::announcement_support::AnnouncementSupportDescriptor,
    ApplicationSignalling = 0x6F => crate::descriptors::application_signalling::ApplicationSignallingDescriptor,
    AdaptationFieldData = 0x70 => crate::descriptors::adaptation_field_data::AdaptationFieldDataDescriptor,
    ServiceIdentifier = 0x71 => crate::descriptors::service_identifier::ServiceIdentifierDescriptor<'a>,
    ServiceAvailability = 0x72 => crate::descriptors::service_availability::ServiceAvailabilityDescriptor,
    DefaultAuthority = 0x73 => crate::descriptors::default_authority::DefaultAuthorityDescriptor<'a>,
    RelatedContent = 0x74 => crate::descriptors::related_content::RelatedContentDescriptor,
    TvaId = 0x75 => crate::descriptors::tva_id::TvaIdDescriptor,
    ContentIdentifier = 0x76 => crate::descriptors::content_identifier::ContentIdentifierDescriptor<'a>,
    TimeSliceFecIdentifier = 0x77 => crate::descriptors::time_slice_fec_identifier::TimeSliceFecIdentifierDescriptor<'a>,
    EcmRepetitionRate = 0x78 => crate::descriptors::ecm_repetition_rate::EcmRepetitionRateDescriptor<'a>,
    S2SatelliteDeliverySystem = 0x79 => crate::descriptors::s2_satellite_delivery_system::S2SatelliteDeliverySystemDescriptor,
    EnhancedAc3 = 0x7A => crate::descriptors::enhanced_ac3::EnhancedAc3Descriptor<'a>,
    Dts = 0x7B => crate::descriptors::dts::DtsDescriptor<'a>,
    Aac = 0x7C => crate::descriptors::aac::AacDescriptor<'a>,
    XaitLocation = 0x7D => crate::descriptors::xait_location::XaitLocationDescriptor,
    FtaContentManagement = 0x7E => crate::descriptors::fta_content_management::FtaContentManagementDescriptor,
    Extension = 0x7F => crate::descriptors::extension::ExtensionDescriptor<'a>;
    // Private / context-dependent: variant exists but is NOT auto-dispatched.
    // 0x83 logical_channel requires private_data_specifier context; enabled
    // via the descriptor registry (Task 4).
    @no_dispatch
    LogicalChannel => crate::descriptors::logical_channel::LogicalChannelDescriptor,
}

/// Lazily walk a raw descriptor loop. Never panics.
///
/// Per-descriptor parse errors yield `Err` and iteration continues (the
/// descriptor_length field bounds each entry, so the walker can always
/// advance past a malformed body). A truncated final header or body yields
/// one `Err` and then the iterator fuses (returns `None` forever after).
#[must_use]
pub fn parse_loop(bytes: &[u8]) -> DescriptorIter<'_> {
    DescriptorIter {
        bytes,
        pos: 0,
        fused: false,
    }
}

/// Iterator over a raw descriptor loop; see [`parse_loop`].
#[derive(Debug, Clone)]
pub struct DescriptorIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
}

impl<'a> Iterator for DescriptorIter<'a> {
    type Item = crate::Result<AnyDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.fused || self.pos >= self.bytes.len() {
            return None;
        }
        let rem = &self.bytes[self.pos..];
        if rem.len() < 2 {
            self.fused = true;
            return Some(Err(crate::Error::BufferTooShort {
                need: 2,
                have: rem.len(),
                what: "descriptor header in loop",
            }));
        }
        let tag = rem[0];
        let len = rem[1] as usize;
        let total = 2 + len;
        if rem.len() < total {
            self.fused = true;
            return Some(Err(crate::Error::BufferTooShort {
                need: total,
                have: rem.len(),
                what: "descriptor body in loop",
            }));
        }
        let full = &rem[..total];
        self.pos += total;
        Some(match AnyDescriptor::dispatch(tag, full) {
            // Ok(typed) or Err(typed parse error) — either way, the length
            // field already advanced `pos`, so iteration continues.
            Some(res) => res,
            None => Ok(AnyDescriptor::Unknown {
                tag,
                body: &full[2..],
            }),
        })
    }
}

impl std::iter::FusedIterator for DescriptorIter<'_> {}

/// A raw descriptor loop, borrowed from the section. Zero-copy: walk it
/// typed via [`DescriptorLoop::iter`]; serde serializes the typed walk.
///
/// This is the table-loop analogue of [`crate::text::DvbText`]: it wraps the
/// raw `descriptor()` sequence (the variable-length region inside a table) and
/// decodes — i.e. dispatches each entry to a typed [`AnyDescriptor`] — only on
/// demand. Parsing stays zero-copy; the typed walk happens when you call
/// [`DescriptorLoop::iter`] or serialize.
///
/// ```
/// use dvb_si::descriptors::{AnyDescriptor, DescriptorLoop};
///
/// // short_event (tag 0x4D, "eng" / "Hi") then an unknown private tag 0xA7.
/// let raw = [
///     0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00,
///     0xA7, 0x02, 0xCA, 0xFE,
/// ];
/// let loop_ = DescriptorLoop::new(&raw);
/// let items: Vec<_> = loop_.iter().collect();
/// assert_eq!(items.len(), 2);
/// assert!(matches!(items[0].as_ref().unwrap(), AnyDescriptor::ShortEvent(_)));
/// assert_eq!(loop_.raw(), &raw[..]); // bytes preserved verbatim
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DescriptorLoop<'a>(&'a [u8]);

impl<'a> DescriptorLoop<'a> {
    /// Wrap a raw descriptor-loop slice (the `descriptor()` bytes only — no
    /// enclosing length field).
    #[must_use]
    pub const fn new(raw: &'a [u8]) -> Self {
        Self(raw)
    }

    /// The raw wire bytes of the loop, verbatim. These are what a serializer
    /// writes back; use them for the byte length of the loop.
    #[must_use]
    pub const fn raw(&self) -> &'a [u8] {
        self.0
    }

    /// Lazily walk the loop, yielding one typed [`AnyDescriptor`] per entry
    /// (or [`AnyDescriptor::Unknown`] for tags with no implementation).
    /// Delegates to [`parse_loop`]; never panics.
    #[must_use]
    pub fn iter(&self) -> DescriptorIter<'a> {
        parse_loop(self.0)
    }
}

impl<'a> std::ops::Deref for DescriptorLoop<'a> {
    /// Derefs to the raw wire bytes — `len()`/indexing are **byte counts for
    /// serialization, not entry counts**. To count entries, use
    /// [`DescriptorLoop::iter`].
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}

impl<'a> From<&'a [u8]> for DescriptorLoop<'a> {
    fn from(raw: &'a [u8]) -> Self {
        Self(raw)
    }
}

impl std::fmt::Debug for DescriptorLoop<'_> {
    /// Cheap: prints the byte length, not the decoded entries.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DescriptorLoop(<{} bytes>)", self.0.len())
    }
}

impl<'a> IntoIterator for &DescriptorLoop<'a> {
    type Item = crate::Result<AnyDescriptor<'a>>;
    type IntoIter = DescriptorIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for DescriptorLoop<'_> {
    /// Serializes as a sequence of the typed walk: each `Ok(d)` becomes the
    /// [`AnyDescriptor`] (camelCase external tagging), and each `Err(e)`
    /// becomes a `{"parseError": "<Display>"}` map — parse errors are surfaced,
    /// never silently dropped.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        struct Entry<'a>(crate::Result<AnyDescriptor<'a>>);
        impl serde::Serialize for Entry<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                match &self.0 {
                    Ok(d) => d.serialize(s),
                    Err(e) => {
                        use serde::ser::SerializeMap;
                        let mut m = s.serialize_map(Some(1))?;
                        m.serialize_entry("parseError", &e.to_string())?;
                        m.end()
                    }
                }
            }
        }
        s.collect_seq(self.iter().map(Entry))
    }
}
// Serialize-only: the typed walk decodes DVB text and dispatches per-tag —
// there is no lossless way to reconstruct the raw loop bytes from the
// serialized form. Structs holding a DescriptorLoop derive Serialize only.
// To reconstruct, keep the wire bytes and re-`parse` the table.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tag_yields_unknown_with_body_sans_header() {
        // tag 0xA7 (no typed impl), length 2, body [0xDE, 0xAD].
        let bytes = [0xA7, 0x02, 0xDE, 0xAD];
        let items: Vec<_> = parse_loop(&bytes).collect();
        assert_eq!(items.len(), 1);
        match items[0].as_ref().unwrap() {
            AnyDescriptor::Unknown { tag, body } => {
                assert_eq!(*tag, 0xA7);
                assert_eq!(*body, &[0xDE, 0xAD]);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn empty_loop_yields_nothing() {
        assert_eq!(parse_loop(&[]).count(), 0);
    }

    #[test]
    fn logical_channel_0x83_is_not_dispatched() {
        // 0x83 has a variant but no dispatcher entry → Unknown, never panics.
        let bytes = [0x83, 0x04, 0x00, 0x01, 0xFC, 0x01];
        let items: Vec<_> = parse_loop(&bytes).collect();
        assert_eq!(items.len(), 1);
        assert!(matches!(
            items[0].as_ref().unwrap(),
            AnyDescriptor::Unknown { tag: 0x83, .. }
        ));
    }

    #[test]
    fn descriptor_loop_iter_matches_parse_loop() {
        let raw = [
            0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
            0xA7, 0x02, 0xCA, 0xFE, // unknown 0xA7
        ];
        let via_loop: Vec<_> = DescriptorLoop::new(&raw)
            .iter()
            .map(|r| format!("{r:?}"))
            .collect();
        let via_fn: Vec<_> = parse_loop(&raw).map(|r| format!("{r:?}")).collect();
        assert_eq!(via_loop, via_fn);
        // raw()/Deref expose the wire bytes (byte length, not entry count).
        assert_eq!(DescriptorLoop::new(&raw).raw(), &raw[..]);
        assert_eq!(DescriptorLoop::new(&raw).len(), raw.len());
        // IntoIterator for &DescriptorLoop.
        let count = (&DescriptorLoop::new(&raw)).into_iter().count();
        assert_eq!(count, 2);
    }

    #[test]
    fn descriptor_loop_debug_is_cheap() {
        let raw = [0x4D, 0x02, 0x01, 0x02];
        assert_eq!(
            format!("{:?}", DescriptorLoop::new(&raw)),
            "DescriptorLoop(<4 bytes>)"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn descriptor_loop_serializes_typed_unknown_and_parse_error() {
        // [valid short_event, unknown tag 0xA7, truncated final entry].
        // A truncated final entry (declared len 5, only 1 body byte present)
        // makes the walker yield a final Err → {"parseError": …}.
        let raw = [
            0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
            0xA7, 0x02, 0xCA, 0xFE, // unknown 0xA7
            0x55, 0x05, 0x00, // parental_rating header claims 5 bytes; only 1 present
        ];
        let v = serde_json::to_value(DescriptorLoop::new(&raw)).unwrap();
        let arr = v.as_array().expect("sequence");
        assert_eq!(arr.len(), 3);
        // 1. typed short_event under the camelCase variant key.
        assert!(arr[0].get("shortEvent").is_some(), "got {}", arr[0]);
        assert_eq!(arr[0]["shortEvent"]["event_name"], "Hi");
        // 2. unknown tag carries its raw body bytes.
        let unknown = arr[1].get("unknown").expect("unknown variant");
        assert_eq!(unknown["tag"], 0xA7);
        assert_eq!(unknown["body"], serde_json::json!([0xCA, 0xFE]));
        // 3. truncated entry → parseError, never silently dropped.
        assert!(
            arr[2].get("parseError").is_some(),
            "expected parseError, got {}",
            arr[2]
        );
    }
}
