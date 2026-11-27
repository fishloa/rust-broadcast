//! Registered four-character identifier enums.
//!
//! `isobmff` types every box, sample entry and property a held specification
//! defines. This module types the *identifier* codes those structures carry as
//! values: brands, handler types, sample-grouping types, track-reference types,
//! track-grouping types, track-selection attributes, metadata key namespaces and
//! restricted scheme types. Each maps a registered four-CC to a named variant and
//! back, so [`crate::coverage`]'s per-table rows can prove their code instead of
//! matching an untyped byte string.
//!
//! The lists are the codes whose defining document is held (ISO/IEC 14496-12:2026,
//! ISO/IEC 14496-15:2017, ISO/IEC 23008-12:2025, ISO/IEC 23009-1:2022,
//! ETSI TS 126 244, QuickTime File Format, AVIF v1.2.0 and the AV1 ISOBMFF
//! binding). A code from a document that is not held stays [`Other`](Brand::Other)
//! and is pinned untyped by the coverage test. Image brands keep their own typed
//! view in [`crate::image_brands::ImageBrand`]; this enum names the brands that
//! view does not.
//!
//! Registration source: `private/specs-verified/mpeg/isobmff/mp4ra/`.

use crate::box_types::BOX_TYPE_SIZE;

/// [`Brand`], [`HandlerType`], [`SampleGroupType`], [`TrackReferenceType`],
/// [`TrackGroupType`], [`TrackSelectionType`], [`KeyNamespace`] and
/// [`RestrictedSchemeType`] share one code-to-variant shape: a registered
/// four-CC, a named variant for every code a held document defines, and an
/// `Other` catch-all that round-trips a code outside that set.
macro_rules! registered_code_enum {
    (
        $(#[$meta:meta])*
        $ty:ident {
            $( $variant:ident => $code:literal, $label:literal, $doc:literal; )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[non_exhaustive]
        pub enum $ty {
            $(
                #[doc = $doc]
                $variant,
            )*
            /// A registered or private code a held document does not define.
            Other([u8; BOX_TYPE_SIZE]),
        }

        impl Default for $ty {
            fn default() -> Self {
                Self::Other([0; BOX_TYPE_SIZE])
            }
        }

        impl $ty {
            /// The code a four-CC names.
            #[must_use]
            pub const fn from_fourcc(code: [u8; BOX_TYPE_SIZE]) -> Self {
                match &code {
                    $( $code => Self::$variant, )*
                    _ => Self::Other(code),
                }
            }

            /// The four-CC this value names.
            #[must_use]
            pub const fn fourcc(self) -> [u8; BOX_TYPE_SIZE] {
                match self {
                    $( Self::$variant => *$code, )*
                    Self::Other(code) => code,
                }
            }

            /// The code a big-endian `unsigned int(32)` field names.
            #[must_use]
            pub const fn from_code(code: u32) -> Self {
                Self::from_fourcc(code.to_be_bytes())
            }

            /// The four-CC as the `unsigned int(32)` a box field carries.
            #[must_use]
            pub const fn code(self) -> u32 {
                u32::from_be_bytes(self.fourcc())
            }

            /// The spec token, per the #204 label convention.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $( Self::$variant => $label, )*
                    Self::Other(_) => "other",
                }
            }
        }

        impl core::fmt::Display for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    Self::Other(code) => write!(
                        f,
                        "{}(0x{:02X}{:02X}{:02X}{:02X})",
                        self.name(),
                        code[0],
                        code[1],
                        code[core::mem::size_of::<u16>()],
                        code[core::mem::size_of::<u32>() - 1]
                    ),
                    other => f.write_str(other.name()),
                }
            }
        }
    };
}

registered_code_enum! {
    /// A registered file brand: a four-CC `ftyp`/`styp` major or compatible brand.
    Brand {
        Avc1 => b"avc1", "avc1", "AVC image brand (ISO/IEC 14496-12:2026)";
        Comp => b"comp", "comp", "Composition brand (ISO/IEC 14496-12:2026)";
        Iso2 => b"iso2", "iso2", "2004 edition brand (ISO/IEC 14496-12:2026)";
        Iso3 => b"iso3", "iso3", "2005 edition brand (ISO/IEC 14496-12:2026)";
        Iso4 => b"iso4", "iso4", "2009 edition brand (ISO/IEC 14496-12:2026)";
        Iso5 => b"iso5", "iso5", "2012 edition brand (ISO/IEC 14496-12:2026)";
        Iso6 => b"iso6", "iso6", "2016 edition brand (ISO/IEC 14496-12:2026)";
        Iso7 => b"iso7", "iso7", "2019 edition brand (ISO/IEC 14496-12:2026)";
        Iso9 => b"iso9", "iso9", "2022 edition brand (ISO/IEC 14496-12:2026)";
        Isoa => b"isoa", "isoa", "Amendment brand (ISO/IEC 14496-12:2026)";
        Isob => b"isob", "isob", "Amendment brand (ISO/IEC 14496-12:2026)";
        Isoc => b"isoc", "isoc", "Amendment brand (ISO/IEC 14496-12:2026)";
        Isod => b"isod", "isod", "Amendment brand (ISO/IEC 14496-12:2026)";
        Isom => b"isom", "isom", "ISO base media file format brand (ISO/IEC 14496-12:2026)";
        Mp71 => b"mp71", "mp71", "MPEG-7 brand (ISO/IEC 14496-12:2026)";
        Relo => b"relo", "relo", "Relocatable brand (ISO/IEC 14496-12:2026)";
        Saie => b"saie", "saie", "Fragment signature brand (ISO/IEC 14496-12:2026)";
        Dash => b"dash", "dash", "DASH brand (ISO/IEC 23009-1:2022)";
        Dsms => b"dsms", "dsms", "DASH self-initialising media segment brand (ISO/IEC 23009-1:2022)";
        Emsg => b"emsg", "emsg", "Event message brand (ISO/IEC 23009-1:2022)";
        Lmsg => b"lmsg", "lmsg", "DASH last media segment brand (ISO/IEC 23009-1:2022)";
        Msdh => b"msdh", "msdh", "DASH general media segment brand (ISO/IEC 23009-1:2022)";
        Msix => b"msix", "msix", "DASH indexed media segment brand (ISO/IEC 23009-1:2022)";
        Risx => b"risx", "risx", "DASH indexed subsegment brand (ISO/IEC 23009-1:2022)";
        Sims => b"sims", "sims", "DASH sub-indexed media segment brand (ISO/IEC 23009-1:2022)";
        Sisx => b"sisx", "sisx", "DASH subsegment index brand (ISO/IEC 23009-1:2022)";
        Ssss => b"ssss", "ssss", "DASH subsegment index segment brand (ISO/IEC 23009-1:2022)";
        Avci => b"avci", "avci", "AVC image brand (ISO/IEC 23008-12:2025)";
        Avcs => b"avcs", "avcs", "AVC image sequence brand (ISO/IEC 23008-12:2025)";
        Evbi => b"evbi", "evbi", "EVC image brand (ISO/IEC 23008-12:2025)";
        Evbs => b"evbs", "evbs", "EVC image sequence brand (ISO/IEC 23008-12:2025)";
        Evmi => b"evmi", "evmi", "EVC image brand (ISO/IEC 23008-12:2025)";
        Evms => b"evms", "evms", "EVC image sequence brand (ISO/IEC 23008-12:2025)";
        Hevc => b"hevc", "hevc", "HEVC image sequence brand (ISO/IEC 23008-12:2025)";
        Hevm => b"hevm", "hevm", "HEVC image sequence brand (ISO/IEC 23008-12:2025)";
        Hevs => b"hevs", "hevs", "HEVC image sequence brand (ISO/IEC 23008-12:2025)";
        Hevx => b"hevx", "hevx", "HEVC image sequence brand (ISO/IEC 23008-12:2025)";
        Jpeg => b"jpeg", "jpeg", "JPEG image brand (ISO/IEC 23008-12:2025)";
        Jpgs => b"jpgs", "jpgs", "JPEG image sequence brand (ISO/IEC 23008-12:2025)";
        Vvic => b"vvic", "vvic", "VVC image brand (ISO/IEC 23008-12:2025)";
        Vvis => b"vvis", "vvis", "VVC image sequence brand (ISO/IEC 23008-12:2025)";
        MA1A => b"MA1A", "MA1A", "AVIF Advanced Profile brand (AVIF v1.2.0)";
        MA1B => b"MA1B", "MA1B", "AVIF Baseline Profile brand (AVIF v1.2.0)";
        Qt => b"qt  ", "qt", "QuickTime brand (QuickTime File Format)";
        Av01 => b"av01", "av01", "AV1 image brand (AV1 ISOBMFF binding)";
        B3ge6 => b"3ge6", "3ge6", "3GPP Release 6 extended-presentation brand (ETSI TS 126 244)";
        B3ge7 => b"3ge7", "3ge7", "3GPP Release 7 extended-presentation brand (ETSI TS 126 244)";
        B3ge9 => b"3ge9", "3ge9", "3GPP Release 9 extended-presentation brand (ETSI TS 126 244)";
        B3gf9 => b"3gf9", "3gf9", "3GPP Release 9 file-delivery server brand (ETSI TS 126 244)";
        B3gg6 => b"3gg6", "3gg6", "3GPP Release 6 general brand (ETSI TS 126 244)";
        B3gg9 => b"3gg9", "3gg9", "3GPP Release 9 general brand (ETSI TS 126 244)";
        B3gh9 => b"3gh9", "3gh9", "3GPP Release 9 brand (ETSI TS 126 244)";
        B3gm9 => b"3gm9", "3gm9", "3GPP Release 9 media brand (ETSI TS 126 244)";
        B3gp4 => b"3gp4", "3gp4", "3GPP Release 4 brand (ETSI TS 126 244)";
        B3gp5 => b"3gp5", "3gp5", "3GPP Release 5 brand (ETSI TS 126 244)";
        B3gp6 => b"3gp6", "3gp6", "3GPP Release 6 brand (ETSI TS 126 244)";
        B3gp7 => b"3gp7", "3gp7", "3GPP Release 7 brand (ETSI TS 126 244)";
        B3gp8 => b"3gp8", "3gp8", "3GPP Release 8 brand (ETSI TS 126 244)";
        B3gp9 => b"3gp9", "3gp9", "3GPP Release 9 brand (ETSI TS 126 244)";
        B3gr6 => b"3gr6", "3gr6", "3GPP Release 6 streaming brand (ETSI TS 126 244)";
        B3gr9 => b"3gr9", "3gr9", "3GPP Release 9 streaming brand (ETSI TS 126 244)";
        B3gs6 => b"3gs6", "3gs6", "3GPP Release 6 brand (ETSI TS 126 244)";
        B3gs9 => b"3gs9", "3gs9", "3GPP Release 9 brand (ETSI TS 126 244)";
        B3gt8 => b"3gt8", "3gt8", "3GPP Release 8 brand (ETSI TS 126 244)";
        B3gt9 => b"3gt9", "3gt9", "3GPP Release 9 brand (ETSI TS 126 244)";
        Ca4e => b"ca4e", "ca4e", "AC-4 subsample-encrypted CMAF media profile brand (ETSI TS 103 190-2)";
        Ca4m => b"ca4m", "ca4m", "AC-4 main CMAF media profile brand (ETSI TS 103 190-2)";
        Ca4s => b"ca4s", "ca4s", "AC-4 single-stream CMAF media profile brand (ETSI TS 103 190-2)";
        Ceac => b"ceac", "ceac", "Enhanced AC-3 CMAF media profile brand (ETSI TS 102 366)";
        Dv58 => b"dv58", "dv58", "Dolby Vision profile 5/8 CMAF media profile brand (Dolby Vision streams in ISOBMFF)";
        Dv09 => b"dv09", "dv09", "Dolby Vision profile 9 brand (Dolby Vision streams in ISOBMFF)";
        Dv10 => b"dv10", "dv10", "Dolby Vision profile 10 brand (Dolby Vision streams in ISOBMFF)";
        Dv20 => b"dv20", "dv20", "Dolby Vision profile 20 brand (Dolby Vision streams in ISOBMFF)";
        Hvce => b"hvce", "hvce", "L-HEVC explicit-reconstruction brand (ISO/IEC 14496-15:2017)";
        Hvci => b"hvci", "hvci", "L-HEVC implicit-reconstruction brand (ISO/IEC 14496-15:2017)";
        Hvcx => b"hvcx", "hvcx", "L-HEVC extended explicit-reconstruction brand (ISO/IEC 14496-15:2017)";
        Hvti => b"hvti", "hvti", "HEVC tile-track brand (ISO/IEC 14496-15:2017)";
        Lhte => b"lhte", "lhte", "L-HEVC tile-track explicit brand (ISO/IEC 14496-15:2017)";
        Lhti => b"lhti", "lhti", "L-HEVC tile-track implicit brand (ISO/IEC 14496-15:2017)";
        Nras => b"nras", "nras", "No leading picture sync brand (ISO/IEC 14496-15:2017)";
    }
}

registered_code_enum! {
    /// A registered handler type: the `handler_type` of a `hdlr` box (§8.4.3.2).
    HandlerType {
        H3gsd => b"3gsd", "3gsd", "3GPP scene description handler (ETSI TS 126 244)";
        Auxv => b"auxv", "auxv", "Auxiliary video handler (ISO/IEC 14496-12:2026 8.4.3)";
        Fdsm => b"fdsm", "fdsm", "Font handler (ISO/IEC 14496-12:2026 8.4.3)";
        Hint => b"hint", "hint", "Hint handler (ISO/IEC 14496-12:2026 8.4.3)";
        Meta => b"meta", "meta", "Metadata handler (ISO/IEC 14496-12:2026 8.4.3)";
        Mp7b => b"mp7b", "mp7b", "MPEG-7 binary metadata handler (ISO/IEC 14496-12:2026 8.4.3)";
        Mp7t => b"mp7t", "mp7t", "MPEG-7 textual metadata handler (ISO/IEC 14496-12:2026 8.4.3)";
        Null => b"null", "null", "No-handling metadata handler (ISO/IEC 14496-12:2026 8.4.3)";
        Soun => b"soun", "soun", "Audio handler (ISO/IEC 14496-12:2026 8.4.3)";
        Subt => b"subt", "subt", "Subtitle handler (ISO/IEC 14496-12:2026 8.4.3)";
        Vide => b"vide", "vide", "Video handler (ISO/IEC 14496-12:2026 8.4.3)";
        Volv => b"volv", "volv", "Volumetric visual media handler (ISO/IEC 14496-12:2026 8.4.3)";
        Hapt => b"hapt", "hapt", "Haptics handler (ISO/IEC 14496-12:2026 8.4.3)";
        Mpd => b"mpd ", "mpd", "MPD metadata handler (ETSI TS 126 244)";
        Mpdl => b"mpdl", "mpdl", "MPD link metadata handler (ETSI TS 126 244)";
        Text => b"text", "text", "Text handler (ETSI TS 126 244)";
        Pict => b"pict", "pict", "Image item and image sequence handler (ISO/IEC 23008-12:2025)";
    }
}

registered_code_enum! {
    /// A registered sample-grouping type: the `grouping_type` of `sbgp`/`sgpd` (§8.9).
    SampleGroupType {
        Alst => b"alst", "alst", "Alternative startup sequence sample group (ISO/IEC 14496-12:2026 10.6)";
        Casg => b"casg", "casg", "Clean aperture sample group (ISO/IEC 14496-12:2026 10.4)";
        Drap => b"drap", "drap", "Dependent random access point sample group (ISO/IEC 14496-12:2026 10.5)";
        Edrp => b"edrp", "edrp", "EDRAP sample group (ISO/IEC 14496-12:2026 10.5)";
        Esgh => b"esgh", "esgh", "Essential descriptions hierarchy sample group (ISO/IEC 14496-12:2026 10.8)";
        It35 => b"it35", "it35", "T.35 sample group (ISO/IEC 14496-12:2026 10.9)";
        Pasr => b"pasr", "pasr", "Pixel aspect ratio sample group (ISO/IEC 14496-12:2026 10.3)";
        Prol => b"prol", "prol", "Audio pre-roll sample group (ISO/IEC 14496-12:2026 10.2)";
        Rap => b"rap ", "rap", "Random access point sample group (ISO/IEC 14496-12:2026 10.4)";
        Rash => b"rash", "rash", "Rate share sample group (ISO/IEC 14496-12:2026 10.10)";
        Roll => b"roll", "roll", "Pre/post roll sample group (ISO/IEC 14496-12:2026 10.1)";
        Sap => b"sap ", "sap", "Stream access point sample group (ISO/IEC 14496-12:2026 10.7)";
        Stmi => b"stmi", "stmi", "Sample-to-metadata-item sample group (ISO/IEC 14496-12:2026)";
        Tele => b"tele", "tele", "Temporal level sample group (ISO/IEC 14496-12:2026 10.5)";
        S3gag => b"3gag", "3gag", "3GPP PSS Annex G video buffer parameters (ETSI TS 126 244)";
        Avcb => b"avcb", "avcb", "AVC HRD parameters (ETSI TS 126 244)";
        Eqiv => b"eqiv", "eqiv", "Visual equivalence sample group (ISO/IEC 23008-12:2025 6.8.1.2)";
        Refs => b"refs", "refs", "Direct reference samples list (ISO/IEC 23008-12:2025 7.4.1)";
        Aebr => b"aebr", "aebr", "Auto exposure bracketing sample group (ISO/IEC 23008-12:2025 6.8.6.2)";
        Afbr => b"afbr", "afbr", "Flash exposure bracketing sample group (ISO/IEC 23008-12:2025 6.8.6.5)";
        Dobr => b"dobr", "dobr", "Depth of field bracketing sample group (ISO/IEC 23008-12:2025 6.8.6.6)";
        Fobr => b"fobr", "fobr", "Focus bracketing sample group (ISO/IEC 23008-12:2025 6.8.6.4)";
        Pano => b"pano", "pano", "Panorama sample group (ISO/IEC 23008-12:2025 6.8.8)";
        Wbbr => b"wbbr", "wbbr", "White balance bracketing sample group (ISO/IEC 23008-12:2025 6.8.6.3)";
        Aud => b"aud ", "aud", "Access unit delimiter sample group (ISO/IEC 14496-15:2017)";
        Avll => b"avll", "avll", "AVC layer sample group (ISO/IEC 14496-15:2017)";
        Avss => b"avss", "avss", "AVC sub-sequence sample group (ISO/IEC 14496-15:2017)";
        Dtrt => b"dtrt", "dtrt", "Decode re-timing sample group (ISO/IEC 14496-15:2017)";
        Eob => b"eob ", "eob", "End of bitstream sample group (ISO/IEC 14496-15:2017)";
        Eos => b"eos ", "eos", "End of sequence sample group (ISO/IEC 14496-15:2017)";
        Linf => b"linf", "linf", "Layer information sample group (ISO/IEC 14496-15:2017)";
        Mvif => b"mvif", "mvif", "Multiview information sample group (ISO/IEC 14496-15:2017)";
        Nalm => b"nalm", "nalm", "NAL unit map sample group (ISO/IEC 14496-15:2017)";
        Pase => b"pase", "pase", "Parameter set sample group (ISO/IEC 14496-15:2017)";
        Scif => b"scif", "scif", "Scalable information sample group (ISO/IEC 14496-15:2017)";
        Scnm => b"scnm", "scnm", "Scalable NAL unit map sample group (ISO/IEC 14496-15:2017)";
        Stsa => b"stsa", "stsa", "Step-wise temporal layer sample group (ISO/IEC 14496-15:2017)";
        Sync => b"sync", "sync", "Sync sample group (ISO/IEC 14496-15:2017)";
        Tscl => b"tscl", "tscl", "Temporal layer sample group (ISO/IEC 14496-15:2017)";
        Vipr => b"vipr", "vipr", "View priority sample group (ISO/IEC 14496-15:2017)";
        Av1f => b"av1f", "av1f", "AV1 forward key frame sample group (AV1 ISOBMFF binding)";
        Av1m => b"av1m", "av1m", "AV1 multi-frame sample group (AV1 ISOBMFF binding)";
        Av1M => b"av1M", "av1M", "AV1 metadata sample group (AV1 ISOBMFF binding)";
        Av1s => b"av1s", "av1s", "AV1 switch frame sample group (AV1 ISOBMFF binding)";
        Regm => b"regm", "regm", "Region annotation map sample group (ISO/IEC 23008-12:2025 11.4.3.1)";
        Udes => b"udes", "udes", "User description sample group (ISO/IEC 23008-12:2025 11.4.3.2)";
    }
}

registered_code_enum! {
    /// A registered track-reference type: the box type of a `tref` child (§8.3.3).
    TrackReferenceType {
        Adda => b"adda", "adda", "Additional audio track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Adrc => b"adrc", "adrc", "DRC metadata track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Auxl => b"auxl", "auxl", "Auxiliary track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Cdsc => b"cdsc", "cdsc", "Describes-the-referenced-track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Font => b"font", "font", "Font track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Hind => b"hind", "hind", "Hint dependency reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Hint => b"hint", "hint", "Hint track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Subt => b"subt", "subt", "Subtitle track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Thmb => b"thmb", "thmb", "Thumbnail track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Vdep => b"vdep", "vdep", "Auxiliary depth video track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Vplx => b"vplx", "vplx", "Auxiliary parallax video track reference (ISO/IEC 14496-12:2026 8.3.3.4)";
        Aest => b"aest", "aest", "Associated external stream track reference (ISO/IEC 14496-12:2026 8.3.3.4.1)";
        Avcp => b"avcp", "avcp", "AVC parameter set stream reference (ISO/IEC 14496-15:2017 5.4.3.3)";
        Deps => b"deps", "deps", "Depth view track reference (ISO/IEC 14496-15:2017)";
        Oref => b"oref", "oref", "Operating point sample group track reference (ISO/IEC 14496-15:2017)";
        Sabt => b"sabt", "sabt", "HEVC tile track reference (ISO/IEC 14496-15:2017)";
        Sbas => b"sbas", "sbas", "Scalable base track reference (ISO/IEC 14496-15:2017)";
        Scal => b"scal", "scal", "Scalable extraction track reference (ISO/IEC 14496-15:2017)";
        Swfr => b"swfr", "swfr", "AVC switch-from track reference (ISO/IEC 14496-15:2017)";
        Swto => b"swto", "swto", "AVC switch-to track reference (ISO/IEC 14496-15:2017)";
        Tbas => b"tbas", "tbas", "HEVC tile base track reference (ISO/IEC 14496-15:2017)";
        Tmcd => b"tmcd", "tmcd", "Timecode track reference (QuickTime File Format)";
        Cdep => b"cdep", "cdep", "Structural dependency track reference (QuickTime File Format)";
        Chap => b"chap", "chap", "Chapter list track reference (QuickTime File Format)";
        Scpt => b"scpt", "scpt", "Transcript track reference (QuickTime File Format)";
        Ssrc => b"ssrc", "ssrc", "Non-primary source track reference (QuickTime File Format)";
        Lyra => b"lyra", "lyra", "Audio layer track dependency reference (ETSI TS 102 114)";
    }
}

registered_code_enum! {
    /// A registered track-grouping type: the box type of a `trgr` child (§8.3.4).
    TrackGroupType {
        Msrc => b"msrc", "msrc", "Multi-source presentation track group (ISO/IEC 14496-12:2026 8.3.4)";
        Ster => b"ster", "ster", "Stereo pair track group (ISO/IEC 14496-12:2026 8.3.4)";
    }
}

registered_code_enum! {
    /// A registered track-selection attribute: a four-CC in a `tsel` attribute list (§8.10.3).
    TrackSelectionType {
        Bitr => b"bitr", "bitr", "Bitrate track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Bwas => b"bwas", "bwas", "Bandwidth track selection attribute (ETSI TS 126 244)";
        Cdec => b"cdec", "cdec", "Codec-type track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Cgsc => b"cgsc", "cgsc", "Coarse-grain-scalable track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Fgsc => b"fgsc", "fgsc", "Fine-grain-scalable track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Frar => b"frar", "frar", "Frame-rate track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Lang => b"lang", "lang", "Language track selection attribute (ETSI TS 126 244)";
        Mela => b"mela", "mela", "Language track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Mpsz => b"mpsz", "mpsz", "Maximum packet size track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Mtyp => b"mtyp", "mtyp", "Media-type track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Nvws => b"nvws", "nvws", "View-count track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Resc => b"resc", "resc", "Region-of-interest-scalable track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Scsz => b"scsz", "scsz", "Size track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Spsc => b"spsc", "spsc", "Spatially-scalable track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Tesc => b"tesc", "tesc", "Temporally-scalable track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
        Tile => b"tile", "tile", "Tile track selection attribute (ISO/IEC 14496-15:2017)";
        Vwsc => b"vwsc", "vwsc", "View-scalable track selection attribute (ISO/IEC 14496-12:2026 8.10.3)";
    }
}

registered_code_enum! {
    /// A registered `keys` entry key namespace (§8.10.7).
    KeyNamespace {
        Mdta => b"mdta", "mdta", "Reverse-DNS key namespace (QuickTime File Format)";
        Me4c => b"me4c", "me4c", "Metadata sample entry four-character key namespace (ISO/IEC 14496-12:2026)";
        Udta => b"udta", "udta", "QuickTime user-data key namespace (QuickTime File Format)";
        Uiso => b"uiso", "uiso", "ISO user-data key namespace (ISO/IEC 14496-12:2026)";
    }
}

registered_code_enum! {
    /// A registered restricted `schm` scheme type (§13.4.6).
    RestrictedSchemeType {
        ASEI => b"aSEI", "aSEI", "SEI-restricted video scheme (ISO/IEC 14496-15:2017)";
        Spkt => b"spkt", "spkt", "Sample-packed track restricted scheme (ISO/IEC 14496-12:2026 13.4.6)";
        Essg => b"essg", "essg", "Essential sample group restricted scheme (ISO/IEC 14496-12:2026 13.4.6)";
        Stvi => b"stvi", "stvi", "Stereoscopic video restricted scheme (ISO/IEC 14496-12:2026 13.5.4)";
        Csch => b"csch", "csch", "Compatible scheme type (ISO/IEC 14496-12:2026 13.4.6)";
    }
}
