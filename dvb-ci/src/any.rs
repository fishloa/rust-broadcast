//! Unified APDU dispatch: [`AnyApdu`].
//!
//! [`AnyApdu`] is generated from a single declarative list (`declare_apdus!`) —
//! one line per resource `apdu_tag`. The list is the single source of truth: it
//! produces the enum, the `From<T>` conversions, the tag → parser dispatcher,
//! and a drift test that pins each tag literal to the type's
//! [`ApduDef::TAG`](crate::traits::ApduDef::TAG). Mirrors dvb-si's
//! `AnyDescriptor` and scte35-splice's `AnyCommand`.
//!
//! An `apdu_tag` with no typed implementation (or one not yet implemented —
//! e.g. the MMI high-level and low-speed-comms objects) falls through to
//! [`AnyApdu::Unknown`], which keeps the raw APDU body so the unit round-trips
//! byte-for-byte.

use crate::error::{Error, Result};
use crate::tag::ApduTag;
use broadcast_common::{Parse, Serialize};

/// Declares [`AnyApdu`] + its dispatcher from one `apdu_tag` list.
macro_rules! declare_apdus {
    (
        $lt:lifetime;
        $( $variant:ident = $($path:ident)::+ $(<$plt:lifetime>)?
            $( [ $( $alt:path ),+ ] )? ),+ $(,)?
    ) => {
        /// Every crate-implemented APDU object, plus an `Unknown` fallthrough
        /// that preserves the raw APDU header + body for lossless round-trips.
        ///
        /// serde uses external tagging with camelCase variant keys.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnyApdu<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// An `apdu_tag` with no typed implementation; the fields are the
            /// raw 3-byte tag and the verbatim body bytes (the `length_value`
            /// bytes that followed the `length_field`).
            Unknown {
                /// The raw 3-byte `apdu_tag`.
                tag: ApduTag,
                /// The raw body bytes.
                #[cfg_attr(feature = "serde", serde(with = "crate::objects::bytes_serde"))]
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyApdu<$lt> {
                fn from(v: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(v)
                }
            }
        )+

        impl<$lt> AnyApdu<$lt> {
            /// Every `apdu_tag` the generated dispatcher routes (excludes
            /// [`AnyApdu::Unknown`]).
            pub const DISPATCHED_TAGS: &'static [ApduTag] = &[
                $( <$($path)::+ as crate::traits::ApduDef>::TAG
                   $(, $( $alt ),+ )? ),+
            ];

            /// Diagnostic SCREAMING_SNAKE name of the contained object
            /// ([`ApduDef::NAME`](crate::traits::ApduDef::NAME)); `"UNKNOWN"` for
            /// [`AnyApdu::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $( Self::$variant(_) =>
                        <$($path)::+ as crate::traits::ApduDef>::NAME, )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// The object's `apdu_tag`.
            #[must_use]
            pub fn tag(&self) -> ApduTag {
                match self {
                    $( Self::$variant(_) =>
                        <$($path)::+ as crate::traits::ApduDef>::TAG, )+
                    Self::Unknown { tag, .. } => *tag,
                }
            }

            /// Parse a complete APDU (header + body) by routing on its 3-byte
            /// `apdu_tag`. Unrecognised tags yield [`AnyApdu::Unknown`].
            pub fn parse(bytes: &$lt [u8]) -> Result<Self> {
                if bytes.len() < 3 {
                    return Err(Error::BufferTooShort { need: 3, have: bytes.len(), what: "apdu_tag" });
                }
                let tag = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
                match tag {
                    $( t if t == <$($path)::+ as crate::traits::ApduDef>::TAG
                        $( || $( t == $alt )||+ )? =>
                        <$($path)::+>::parse(bytes).map(Self::$variant), )+
                    _ => {
                        let body = crate::objects::parse_apdu_header(bytes, tag, "unknown apdu")?;
                        Ok(Self::Unknown { tag, body })
                    }
                }
            }
        }

        impl Serialize for AnyApdu<'_> {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                match self {
                    $( Self::$variant(v) => v.serialized_len(), )+
                    Self::Unknown { body, .. } => crate::objects::apdu_len(body.len()),
                }
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                match self {
                    $( Self::$variant(v) => v.serialize_into(buf), )+
                    Self::Unknown { tag, body } => {
                        let mut pos = crate::objects::write_apdu_header(*tag, body.len(), buf)?;
                        buf[pos..pos + body.len()].copy_from_slice(body);
                        pos += body.len();
                        Ok(pos)
                    }
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn tag_literals_match_apdu_def() {
                use crate::traits::ApduDef;
                $(
                    assert!(
                        !<$($path)::+ as ApduDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                    // The list carries no separate literal: the dispatcher and
                    // DISPATCHED_TAGS both read ApduDef::TAG, so this asserts the
                    // tag is a public 0x9F-prefixed tag (Figure 16).
                    assert_eq!(
                        <$($path)::+ as ApduDef>::TAG.to_bytes()[0],
                        crate::tag::APDU_TAG_PREFIX,
                        concat!("non-0x9F apdu_tag for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_apdus! {'a;
    ProfileEnq          = crate::objects::resource_manager::ProfileEnq,
    Profile             = crate::objects::resource_manager::Profile,
    ProfileChange       = crate::objects::resource_manager::ProfileChange,
    ApplicationInfoEnq  = crate::objects::application_info::ApplicationInfoEnq,
    ApplicationInfo     = crate::objects::application_info::ApplicationInfo<'a>,
    EnterMenu           = crate::objects::application_info::EnterMenu,
    CaInfoEnq           = crate::objects::ca_info::CaInfoEnq,
    CaInfo              = crate::objects::ca_info::CaInfo,
    CaPmt               = crate::objects::ca_pmt::CaPmt<'a>,
    CaPmtReply          = crate::objects::ca_pmt_reply::CaPmtReply,
    DateTimeEnq         = crate::objects::date_time::DateTimeEnq,
    DateTime            = crate::objects::date_time::DateTime,
    CloseMmi            = crate::objects::mmi_close::CloseMmi,
    // Host Control (§8.5).
    Tune                = crate::objects::host_control::Tune,
    Replace             = crate::objects::host_control::Replace,
    ClearReplace        = crate::objects::host_control::ClearReplace,
    AskRelease          = crate::objects::host_control::AskRelease,
    // High-level MMI (§8.6.5).
    Text                = crate::objects::mmi_high::Text<'a>         [crate::tag::TEXT_MORE],
    Enq                 = crate::objects::mmi_high::Enq<'a>,
    Answ                = crate::objects::mmi_high::Answ<'a>,
    Menu                = crate::objects::mmi_high::Menu<'a>         [crate::tag::MENU_MORE],
    MenuAnsw            = crate::objects::mmi_high::MenuAnsw,
    List                = crate::objects::mmi_high::List<'a>         [crate::tag::LIST_MORE],
    // Low-level / display / scene / download MMI (§8.6.2-8.6.4).
    DisplayControl      = crate::objects::mmi_display::DisplayControl,
    DisplayReply        = crate::objects::mmi_display::DisplayReply,
    KeypadControl       = crate::objects::mmi_display::KeypadControl,
    Keypress            = crate::objects::mmi_display::Keypress,
    SubtitleSegment     = crate::objects::mmi_display::SubtitleSegment<'a>  [crate::tag::SUBTITLE_SEGMENT_MORE],
    DisplayMessage      = crate::objects::mmi_display::DisplayMessage,
    SceneEndMark        = crate::objects::mmi_display::SceneEndMark,
    SceneDoneMessage    = crate::objects::mmi_display::SceneDoneMessage,
    SceneControl        = crate::objects::mmi_display::SceneControl,
    SubtitleDownload    = crate::objects::mmi_display::SubtitleDownload<'a> [crate::tag::SUBTITLE_DOWNLOAD_MORE],
    FlushDownload       = crate::objects::mmi_display::FlushDownload,
    DownloadReply       = crate::objects::mmi_display::DownloadReply,
    // Low-speed comms (§8.7).
    CommsCmd            = crate::objects::low_speed_comms::CommsCmd<'a>,
    ConnectionDescriptor = crate::objects::low_speed_comms::ConnectionDescriptor<'a>,
    CommsReply          = crate::objects::low_speed_comms::CommsReply,
    CommsSend           = crate::objects::low_speed_comms::CommsSend<'a>    [crate::tag::COMMS_SEND_MORE],
    CommsRcv            = crate::objects::low_speed_comms::CommsRcv<'a>     [crate::tag::COMMS_RCV_MORE],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::ca_pmt::{CaPmt, CaPmtListManagement};

    #[test]
    fn dispatch_ca_pmt() {
        let pmt = CaPmt {
            list_management: CaPmtListManagement::Only,
            program_number: 1,
            version_number: 0,
            current_next_indicator: true,
            cmd_id: None,
            program_ca_descriptors: &[],
            streams: alloc::vec::Vec::new(),
        };
        let bytes = pmt.to_bytes();
        let any = AnyApdu::parse(&bytes).unwrap();
        assert_eq!(any.name(), "CA_PMT");
        assert_eq!(any.tag(), crate::tag::CA_PMT);
        // round-trips through AnyApdu.
        assert_eq!(any.to_bytes(), bytes);
    }

    #[test]
    fn unknown_tag_round_trips() {
        // Pick a private/unallocated tag (not in Table 58).
        let bytes = [0x9F, 0x90, 0x01, 0x02, 0xAA, 0xBB];
        let any = AnyApdu::parse(&bytes).unwrap();
        assert!(matches!(any, AnyApdu::Unknown { .. }));
        assert_eq!(any.name(), "UNKNOWN");
        assert_eq!(any.to_bytes(), bytes);
    }

    #[test]
    fn dispatches_more_tag_to_same_variant() {
        use crate::objects::mmi_high::Text;
        // text_more (9F8804) must dispatch to the Text variant, not Unknown.
        let t = Text {
            more: true,
            text_chars: b"HI",
        };
        let bytes = t.to_bytes();
        assert_eq!(bytes[2], 0x04);
        let any = AnyApdu::parse(&bytes).unwrap();
        assert_eq!(any.name(), "TEXT");
        assert_eq!(any.to_bytes(), bytes);
    }

    #[test]
    fn dispatched_tags_listed() {
        assert!(AnyApdu::DISPATCHED_TAGS.contains(&crate::tag::CA_PMT));
        assert!(AnyApdu::DISPATCHED_TAGS.contains(&crate::tag::PROFILE_ENQ));
        assert!(AnyApdu::DISPATCHED_TAGS.contains(&crate::tag::TEXT_MORE));
        assert!(AnyApdu::DISPATCHED_TAGS.contains(&crate::tag::COMMS_SEND_MORE));
        // 40 primary tags (one per typed object) + 7 alt (_more) chaining tags.
        assert_eq!(AnyApdu::DISPATCHED_TAGS.len(), 40 + 7);
    }
}
