//! MP4RA registration coverage: every code in every table the MP4 Registration
//! Authority publishes, partitioned into typed, pinned untyped (with and
//! without a held specification) and not-applicable.
//!
//! The tables are generated from `private/specs-verified/mpeg/isobmff/mp4ra/`
//! and pinned here so `tests/it/mp4ra.rs` can check the partition against the
//! CSVs. A typed row proves itself: a structured code parses a minimal instance
//! through the crate's parser, an enumerated identifier maps through its typed
//! enum and back. A code in no list, or in two, fails the test.
//!
//! A specification counts as held only when it is under `private/specs/` or
//! transcribed under `private/specs-verified/`. A code whose defining document
//! is not held is pinned as such rather than implemented from memory.
//!
//! Regenerate with `tools/gen-mp4ra-coverage.py` after editing the typed sets
//! or adding a held document.

/// One code a typed row names.
#[derive(Debug, Clone, Copy)]
pub struct TypedRow {
    /// The four-CC.
    pub code: &'static str,
}

/// A code that is not a structure this crate parses, with the reason.
#[derive(Debug, Clone, Copy)]
pub struct NotApplicable {
    /// The four-CC or table value.
    pub code: &'static str,
    /// Why the code is not an ISOBMFF structure.
    pub reason: &'static str,
}

/// The kind of identifier a typed table names, which selects its proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedKind {
    /// A registered box.
    Box,
    /// A QuickTime atom outside the ISO box set.
    QtAtom,
    /// A QuickTime/3GPP user-data atom.
    UdtaAtom,
    /// A sample entry.
    SampleEntry,
    /// A configuration or extension box inside a sample entry.
    SampleEntryBox,
    /// An item property.
    ItemProperty,
    /// An item type.
    ItemType,
    /// An item reference type.
    Reference,
    /// An entity-grouping type.
    EntityGroup,
    /// A sample-grouping type.
    SampleGroup,
    /// A track-grouping type.
    TrackGroup,
    /// A track-reference type.
    TrackReference,
    /// A track-selection attribute.
    TrackSelection,
    /// A metadata key namespace.
    KeyNamespace,
    /// A restricted or protection scheme type.
    Scheme,
    /// A sample-auxiliary-information type.
    AuxInfoType,
    /// A colour type.
    ColourType,
    /// A data-reference entry type.
    DataReference,
    /// A file brand.
    Brand,
    /// A handler type.
    Handler,
}

impl TypedKind {
    /// The typed table this kind reads.
    #[must_use]
    pub fn rows(self) -> &'static [TypedRow] {
        match self {
            Self::Box => TYPED_BOXES_MP4RA,
            Self::QtAtom => TYPED_QT_ATOMS,
            Self::UdtaAtom => TYPED_UDTA_ATOMS,
            Self::SampleEntry => TYPED_SAMPLE_ENTRIES,
            Self::SampleEntryBox => TYPED_SAMPLE_ENTRY_BOXES,
            Self::ItemProperty => TYPED_ITEM_PROPERTIES,
            Self::ItemType => TYPED_ITEM_TYPES,
            Self::Reference => TYPED_REFERENCES,
            Self::EntityGroup => TYPED_ENTITY_GROUPS,
            Self::SampleGroup => TYPED_SAMPLE_GROUPS,
            Self::TrackGroup => TYPED_TRACK_GROUPS,
            Self::TrackReference => TYPED_TRACK_REFERENCES,
            Self::TrackSelection => TYPED_TRACK_SELECTION,
            Self::KeyNamespace => TYPED_KEY_NAMESPACES,
            Self::Scheme => TYPED_SCHEMES,
            Self::AuxInfoType => TYPED_AUX_INFO_TYPES,
            Self::ColourType => TYPED_COLOUR_TYPES,
            Self::DataReference => TYPED_DATA_REFERENCES,
            Self::Brand => KNOWN_BRANDS,
            Self::Handler => KNOWN_HANDLERS,
        }
    }
}

/// The Box codes a held document defines and this crate types.
pub const TYPED_BOXES_MP4RA: &[TypedRow] = &[
    TypedRow { code: "!mof" },
    TypedRow { code: "!mov" },
    TypedRow { code: "!six" },
    TypedRow { code: "!ssx" },
    TypedRow { code: "3gau" },
    TypedRow { code: "aedb" },
    TypedRow { code: "aelm" },
    TypedRow { code: "aepp" },
    TypedRow { code: "aepr" },
    TypedRow { code: "aesb" },
    TypedRow { code: "aesd" },
    TypedRow { code: "amve" },
    TypedRow { code: "ardi" },
    TypedRow { code: "assp" },
    TypedRow { code: "brnd" },
    TypedRow { code: "btrt" },
    TypedRow { code: "bxml" },
    TypedRow { code: "cclv" },
    TypedRow { code: "ccst" },
    TypedRow { code: "chnl" },
    TypedRow { code: "cinf" },
    TypedRow { code: "clap" },
    TypedRow { code: "clli" },
    TypedRow { code: "co64" },
    TypedRow { code: "colr" },
    TypedRow { code: "cprt" },
    TypedRow { code: "csch" },
    TypedRow { code: "csgp" },
    TypedRow { code: "cslg" },
    TypedRow { code: "ctts" },
    TypedRow { code: "dimm" },
    TypedRow { code: "dinf" },
    TypedRow { code: "dmax" },
    TypedRow { code: "dmed" },
    TypedRow { code: "dmix" },
    TypedRow { code: "dref" },
    TypedRow { code: "drep" },
    TypedRow { code: "edts" },
    TypedRow { code: "elng" },
    TypedRow { code: "elst" },
    TypedRow { code: "emsg" },
    TypedRow { code: "etyp" },
    TypedRow { code: "extr" },
    TypedRow { code: "fdpa" },
    TypedRow { code: "fdsa" },
    TypedRow { code: "feci" },
    TypedRow { code: "fecr" },
    TypedRow { code: "fiin" },
    TypedRow { code: "fire" },
    TypedRow { code: "fpar" },
    TypedRow { code: "frma" },
    TypedRow { code: "ftyp" },
    TypedRow { code: "gitn" },
    TypedRow { code: "grpl" },
    TypedRow { code: "hdlr" },
    TypedRow { code: "hinf" },
    TypedRow { code: "hmhd" },
    TypedRow { code: "hnti" },
    TypedRow { code: "idat" },
    TypedRow { code: "iinf" },
    TypedRow { code: "iloc" },
    TypedRow { code: "imda" },
    TypedRow { code: "infe" },
    TypedRow { code: "ipco" },
    TypedRow { code: "ipma" },
    TypedRow { code: "ipro" },
    TypedRow { code: "iprp" },
    TypedRow { code: "iref" },
    TypedRow { code: "istm" },
    TypedRow { code: "keyd" },
    TypedRow { code: "keys" },
    TypedRow { code: "kind" },
    TypedRow { code: "labl" },
    TypedRow { code: "leva" },
    TypedRow { code: "loca" },
    TypedRow { code: "ludt" },
    TypedRow { code: "maxr" },
    TypedRow { code: "md5i" },
    TypedRow { code: "mdat" },
    TypedRow { code: "mdcv" },
    TypedRow { code: "mdhd" },
    TypedRow { code: "mdia" },
    TypedRow { code: "mehd" },
    TypedRow { code: "meta" },
    TypedRow { code: "mfhd" },
    TypedRow { code: "mfra" },
    TypedRow { code: "mfro" },
    TypedRow { code: "minf" },
    TypedRow { code: "moof" },
    TypedRow { code: "moov" },
    TypedRow { code: "mvex" },
    TypedRow { code: "mvhd" },
    TypedRow { code: "nmhd" },
    TypedRow { code: "npck" },
    TypedRow { code: "nump" },
    TypedRow { code: "otyp" },
    TypedRow { code: "padb" },
    TypedRow { code: "paen" },
    TypedRow { code: "pasp" },
    TypedRow { code: "payt" },
    TypedRow { code: "pdin" },
    TypedRow { code: "pitm" },
    TypedRow { code: "pmax" },
    TypedRow { code: "prft" },
    TypedRow { code: "pssh" },
    TypedRow { code: "rcsr" },
    TypedRow { code: "rinf" },
    TypedRow { code: "rsot" },
    TypedRow { code: "rssr" },
    TypedRow { code: "rtp" },
    TypedRow { code: "rtpx" },
    TypedRow { code: "saio" },
    TypedRow { code: "saiz" },
    TypedRow { code: "sbgp" },
    TypedRow { code: "schi" },
    TypedRow { code: "schm" },
    TypedRow { code: "scrb" },
    TypedRow { code: "sdp" },
    TypedRow { code: "sdtp" },
    TypedRow { code: "segr" },
    TypedRow { code: "senc" },
    TypedRow { code: "setu" },
    TypedRow { code: "sgpd" },
    TypedRow { code: "sidx" },
    TypedRow { code: "sinf" },
    TypedRow { code: "smhd" },
    TypedRow { code: "snro" },
    TypedRow { code: "spdb" },
    TypedRow { code: "spki" },
    TypedRow { code: "srat" },
    TypedRow { code: "srpp" },
    TypedRow { code: "ssix" },
    TypedRow { code: "ssrt" },
    TypedRow { code: "stbl" },
    TypedRow { code: "stco" },
    TypedRow { code: "stdp" },
    TypedRow { code: "sthd" },
    TypedRow { code: "strd" },
    TypedRow { code: "stri" },
    TypedRow { code: "strk" },
    TypedRow { code: "stsc" },
    TypedRow { code: "stsd" },
    TypedRow { code: "stsg" },
    TypedRow { code: "stsh" },
    TypedRow { code: "stss" },
    TypedRow { code: "stsz" },
    TypedRow { code: "stts" },
    TypedRow { code: "stvi" },
    TypedRow { code: "styp" },
    TypedRow { code: "stz2" },
    TypedRow { code: "subs" },
    TypedRow { code: "tPAT" },
    TypedRow { code: "tPMT" },
    TypedRow { code: "tenc" },
    TypedRow { code: "tfad" },
    TypedRow { code: "tfdt" },
    TypedRow { code: "tfhd" },
    TypedRow { code: "tfma" },
    TypedRow { code: "tfra" },
    TypedRow { code: "tims" },
    TypedRow { code: "tkhd" },
    TypedRow { code: "tmax" },
    TypedRow { code: "tmin" },
    TypedRow { code: "totl" },
    TypedRow { code: "tpay" },
    TypedRow { code: "tpyl" },
    TypedRow { code: "traf" },
    TypedRow { code: "trak" },
    TypedRow { code: "tran" },
    TypedRow { code: "tref" },
    TypedRow { code: "trep" },
    TypedRow { code: "trex" },
    TypedRow { code: "trgr" },
    TypedRow { code: "trpy" },
    TypedRow { code: "trun" },
    TypedRow { code: "tsel" },
    TypedRow { code: "tsro" },
    TypedRow { code: "tssy" },
    TypedRow { code: "tsti" },
    TypedRow { code: "ttyp" },
    TypedRow { code: "tyco" },
    TypedRow { code: "udta" },
    TypedRow { code: "uri" },
    TypedRow { code: "uriI" },
    TypedRow { code: "url" },
    TypedRow { code: "vmhd" },
    TypedRow { code: "vvhd" },
    TypedRow { code: "wide" },
    TypedRow { code: "xml" },
];

/// The QtAtom codes a held document defines and this crate types.
pub const TYPED_QT_ATOMS: &[TypedRow] = &[
    TypedRow { code: "albm" },
    TypedRow { code: "auth" },
    TypedRow { code: "cprt" },
    TypedRow { code: "elng" },
    TypedRow { code: "load" },
];

/// The UdtaAtom codes a held document defines and this crate types.
pub const TYPED_UDTA_ATOMS: &[TypedRow] = &[
    TypedRow { code: "albm" },
    TypedRow { code: "alou" },
    TypedRow { code: "auth" },
    TypedRow { code: "clsf" },
    TypedRow { code: "coll" },
    TypedRow { code: "cprt" },
    TypedRow { code: "dscp" },
    TypedRow { code: "gnre" },
    TypedRow { code: "hinf" },
    TypedRow { code: "hnti" },
    TypedRow { code: "kind" },
    TypedRow { code: "kywd" },
    TypedRow { code: "loci" },
    TypedRow { code: "ludt" },
    TypedRow { code: "orie" },
    TypedRow { code: "perf" },
    TypedRow { code: "rtng" },
    TypedRow { code: "strk" },
    TypedRow { code: "thmb" },
    TypedRow { code: "titl" },
    TypedRow { code: "tlou" },
    TypedRow { code: "tsel" },
    TypedRow { code: "urat" },
    TypedRow { code: "yrrc" },
];

/// The SampleEntry codes a held document defines and this crate types.
pub const TYPED_SAMPLE_ENTRIES: &[TypedRow] = &[
    TypedRow { code: "3glo" },
    TypedRow { code: "3gor" },
    TypedRow { code: "3gvo" },
    TypedRow { code: "NONE" },
    TypedRow { code: "Opus" },
    TypedRow { code: "PNTG" },
    TypedRow { code: "QDM2" },
    TypedRow { code: "QDMC" },
    TypedRow { code: "Qclp" },
    TypedRow { code: "SVQ1" },
    TypedRow { code: "SVQ3" },
    TypedRow { code: "WRLE" },
    TypedRow { code: "ac-3" },
    TypedRow { code: "ac-4" },
    TypedRow { code: "agsm" },
    TypedRow { code: "alaw" },
    TypedRow { code: "ap4h" },
    TypedRow { code: "ap4x" },
    TypedRow { code: "apch" },
    TypedRow { code: "apcn" },
    TypedRow { code: "apco" },
    TypedRow { code: "apcs" },
    TypedRow { code: "aprh" },
    TypedRow { code: "aprn" },
    TypedRow { code: "av01" },
    TypedRow { code: "avc1" },
    TypedRow { code: "avc2" },
    TypedRow { code: "avc3" },
    TypedRow { code: "avc4" },
    TypedRow { code: "avcp" },
    TypedRow { code: "avs3" },
    TypedRow { code: "c608" },
    TypedRow { code: "c708" },
    TypedRow { code: "civd" },
    TypedRow { code: "dtsc" },
    TypedRow { code: "dtse" },
    TypedRow { code: "dtsh" },
    TypedRow { code: "dtsl" },
    TypedRow { code: "dvh1" },
    TypedRow { code: "dvh5" },
    TypedRow { code: "dvh6" },
    TypedRow { code: "dvhe" },
    TypedRow { code: "dvhp" },
    TypedRow { code: "dvi" },
    TypedRow { code: "ec-3" },
    TypedRow { code: "enca" },
    TypedRow { code: "encv" },
    TypedRow { code: "fLaC" },
    TypedRow { code: "fdp" },
    TypedRow { code: "fl32" },
    TypedRow { code: "fl64" },
    TypedRow { code: "flic" },
    TypedRow { code: "gif" },
    TypedRow { code: "h261" },
    TypedRow { code: "h263" },
    TypedRow { code: "hev1" },
    TypedRow { code: "hvc1" },
    TypedRow { code: "ima4" },
    TypedRow { code: "in24" },
    TypedRow { code: "in32" },
    TypedRow { code: "jpeg" },
    TypedRow { code: "lav3" },
    TypedRow { code: "lpcm" },
    TypedRow { code: "mebx" },
    TypedRow { code: "mett" },
    TypedRow { code: "metx" },
    TypedRow { code: "mha1" },
    TypedRow { code: "mha2" },
    TypedRow { code: "mhm1" },
    TypedRow { code: "mhm2" },
    TypedRow { code: "mjpa" },
    TypedRow { code: "mjpb" },
    TypedRow { code: "mp4a" },
    TypedRow { code: "mp4v" },
    TypedRow { code: "pm2t" },
    TypedRow { code: "png" },
    TypedRow { code: "prtp" },
    TypedRow { code: "raw" },
    TypedRow { code: "rgan" },
    TypedRow { code: "rle" },
    TypedRow { code: "rm2t" },
    TypedRow { code: "rpza" },
    TypedRow { code: "rrtp" },
    TypedRow { code: "rsrp" },
    TypedRow { code: "rtcp" },
    TypedRow { code: "rtp" },
    TypedRow { code: "s263" },
    TypedRow { code: "samr" },
    TypedRow { code: "sawb" },
    TypedRow { code: "sawp" },
    TypedRow { code: "sevs" },
    TypedRow { code: "sm2t" },
    TypedRow { code: "sowt" },
    TypedRow { code: "srtp" },
    TypedRow { code: "stcp" },
    TypedRow { code: "stpp" },
    TypedRow { code: "stxt" },
    TypedRow { code: "tga" },
    TypedRow { code: "tiff" },
    TypedRow { code: "tmcd" },
    TypedRow { code: "twos" },
    TypedRow { code: "ulaw" },
    TypedRow { code: "urim" },
    TypedRow { code: "vdva" },
    TypedRow { code: "vp09" },
    TypedRow { code: "vvc1" },
    TypedRow { code: "vvi1" },
    TypedRow { code: "wvtt" },
];

/// The SampleEntryBox codes a held document defines and this crate types.
pub const TYPED_SAMPLE_ENTRY_BOXES: &[TypedRow] = &[
    TypedRow { code: "amve" },
    TypedRow { code: "av1C" },
    TypedRow { code: "av2c" },
    TypedRow { code: "av3c" },
    TypedRow { code: "avcC" },
    TypedRow { code: "btrt" },
    TypedRow { code: "cclv" },
    TypedRow { code: "ccst" },
    TypedRow { code: "chnl" },
    TypedRow { code: "clap" },
    TypedRow { code: "clli" },
    TypedRow { code: "colr" },
    TypedRow { code: "d263" },
    TypedRow { code: "dOps" },
    TypedRow { code: "dac3" },
    TypedRow { code: "dac4" },
    TypedRow { code: "damr" },
    TypedRow { code: "dawp" },
    TypedRow { code: "dca3" },
    TypedRow { code: "ddts" },
    TypedRow { code: "dec3" },
    TypedRow { code: "dfLa" },
    TypedRow { code: "dmix" },
    TypedRow { code: "dvcC" },
    TypedRow { code: "dvvC" },
    TypedRow { code: "esds" },
    TypedRow { code: "hvcC" },
    TypedRow { code: "la2c" },
    TypedRow { code: "lavc" },
    TypedRow { code: "mdcv" },
    TypedRow { code: "mhaC" },
    TypedRow { code: "pasp" },
    TypedRow { code: "scrb" },
    TypedRow { code: "srat" },
    TypedRow { code: "txtC" },
    TypedRow { code: "uriI" },
    TypedRow { code: "vpcC" },
    TypedRow { code: "vttC" },
    TypedRow { code: "vvcC" },
];

/// The ItemProperty codes a held document defines and this crate types.
pub const TYPED_ITEM_PROPERTIES: &[TypedRow] = &[
    TypedRow { code: "a1lx" },
    TypedRow { code: "a1op" },
    TypedRow { code: "aebr" },
    TypedRow { code: "afbr" },
    TypedRow { code: "altt" },
    TypedRow { code: "amve" },
    TypedRow { code: "auxC" },
    TypedRow { code: "av1C" },
    TypedRow { code: "avcC" },
    TypedRow { code: "brnd" },
    TypedRow { code: "cclv" },
    TypedRow { code: "cexg" },
    TypedRow { code: "clap" },
    TypedRow { code: "clli" },
    TypedRow { code: "cmex" },
    TypedRow { code: "cmin" },
    TypedRow { code: "colr" },
    TypedRow { code: "crtt" },
    TypedRow { code: "dadj" },
    TypedRow { code: "dobr" },
    TypedRow { code: "fade" },
    TypedRow { code: "fobr" },
    TypedRow { code: "hdlp" },
    TypedRow { code: "hvcC" },
    TypedRow { code: "imir" },
    TypedRow { code: "irot" },
    TypedRow { code: "iscl" },
    TypedRow { code: "ispe" },
    TypedRow { code: "lsel" },
    TypedRow { code: "mdcv" },
    TypedRow { code: "mdft" },
    TypedRow { code: "mskC" },
    TypedRow { code: "ndwt" },
    TypedRow { code: "pano" },
    TypedRow { code: "pasp" },
    TypedRow { code: "pixi" },
    TypedRow { code: "prdi" },
    TypedRow { code: "reve" },
    TypedRow { code: "rloc" },
    TypedRow { code: "rref" },
    TypedRow { code: "scrb" },
    TypedRow { code: "splt" },
    TypedRow { code: "ssld" },
    TypedRow { code: "sstr" },
    TypedRow { code: "stag" },
    TypedRow { code: "stpe" },
    TypedRow { code: "stvi" },
    TypedRow { code: "subs" },
    TypedRow { code: "tols" },
    TypedRow { code: "udes" },
    TypedRow { code: "vvcC" },
    TypedRow { code: "wbbr" },
    TypedRow { code: "wipe" },
    TypedRow { code: "zoom" },
];

/// The ItemType codes a held document defines and this crate types.
pub const TYPED_ITEM_TYPES: &[TypedRow] = &[
    TypedRow { code: "Exif" },
    TypedRow { code: "auvd" },
    TypedRow { code: "av01" },
    TypedRow { code: "avc1" },
    TypedRow { code: "cfen" },
    TypedRow { code: "evc1" },
    TypedRow { code: "fdel" },
    TypedRow { code: "grid" },
    TypedRow { code: "hvc1" },
    TypedRow { code: "hvt1" },
    TypedRow { code: "iden" },
    TypedRow { code: "iovl" },
    TypedRow { code: "it35" },
    TypedRow { code: "jpeg" },
    TypedRow { code: "lhv1" },
    TypedRow { code: "mime" },
    TypedRow { code: "mint" },
    TypedRow { code: "mski" },
    TypedRow { code: "rgan" },
    TypedRow { code: "sato" },
    TypedRow { code: "tmap" },
    TypedRow { code: "uri" },
    TypedRow { code: "vvc1" },
];

/// The Reference codes a held document defines and this crate types.
pub const TYPED_REFERENCES: &[TypedRow] = &[
    TypedRow { code: "anot" },
    TypedRow { code: "auxl" },
    TypedRow { code: "base" },
    TypedRow { code: "cdsc" },
    TypedRow { code: "dimg" },
    TypedRow { code: "dpnd" },
    TypedRow { code: "drgn" },
    TypedRow { code: "eroi" },
    TypedRow { code: "exbl" },
    TypedRow { code: "fdel" },
    TypedRow { code: "font" },
    TypedRow { code: "iloc" },
    TypedRow { code: "mask" },
    TypedRow { code: "mint" },
    TypedRow { code: "pred" },
    TypedRow { code: "prem" },
    TypedRow { code: "rpds" },
    TypedRow { code: "tbas" },
    TypedRow { code: "thmb" },
];

/// The EntityGroup codes a held document defines and this crate types.
pub const TYPED_ENTITY_GROUPS: &[TypedRow] = &[
    TypedRow { code: "aebr" },
    TypedRow { code: "afbr" },
    TypedRow { code: "albc" },
    TypedRow { code: "altr" },
    TypedRow { code: "brst" },
    TypedRow { code: "dobr" },
    TypedRow { code: "eqiv" },
    TypedRow { code: "favc" },
    TypedRow { code: "fobr" },
    TypedRow { code: "iaug" },
    TypedRow { code: "opeg" },
    TypedRow { code: "pano" },
    TypedRow { code: "prgr" },
    TypedRow { code: "prsl" },
    TypedRow { code: "pymd" },
    TypedRow { code: "rgpa" },
    TypedRow { code: "slid" },
    TypedRow { code: "stem" },
    TypedRow { code: "ster" },
    TypedRow { code: "tsyn" },
    TypedRow { code: "wbbr" },
];

/// The SampleGroup codes a held document defines and this crate types.
pub const TYPED_SAMPLE_GROUPS: &[TypedRow] = &[
    TypedRow { code: "3gag" },
    TypedRow { code: "aebr" },
    TypedRow { code: "afbr" },
    TypedRow { code: "alst" },
    TypedRow { code: "aud" },
    TypedRow { code: "av1M" },
    TypedRow { code: "av1f" },
    TypedRow { code: "av1m" },
    TypedRow { code: "av1s" },
    TypedRow { code: "avcb" },
    TypedRow { code: "avll" },
    TypedRow { code: "avss" },
    TypedRow { code: "casg" },
    TypedRow { code: "dobr" },
    TypedRow { code: "drap" },
    TypedRow { code: "dtrt" },
    TypedRow { code: "edrp" },
    TypedRow { code: "eob" },
    TypedRow { code: "eos" },
    TypedRow { code: "eqiv" },
    TypedRow { code: "esgh" },
    TypedRow { code: "fobr" },
    TypedRow { code: "it35" },
    TypedRow { code: "linf" },
    TypedRow { code: "mvif" },
    TypedRow { code: "nalm" },
    TypedRow { code: "pano" },
    TypedRow { code: "pase" },
    TypedRow { code: "pasr" },
    TypedRow { code: "prol" },
    TypedRow { code: "rap" },
    TypedRow { code: "rash" },
    TypedRow { code: "refs" },
    TypedRow { code: "regm" },
    TypedRow { code: "roll" },
    TypedRow { code: "sap" },
    TypedRow { code: "scif" },
    TypedRow { code: "scnm" },
    TypedRow { code: "stmi" },
    TypedRow { code: "stsa" },
    TypedRow { code: "sync" },
    TypedRow { code: "tele" },
    TypedRow { code: "tscl" },
    TypedRow { code: "udes" },
    TypedRow { code: "vipr" },
    TypedRow { code: "wbbr" },
];

/// The TrackGroup codes a held document defines and this crate types.
pub const TYPED_TRACK_GROUPS: &[TypedRow] = &[TypedRow { code: "msrc" }, TypedRow { code: "ster" }];

/// The TrackReference codes a held document defines and this crate types.
pub const TYPED_TRACK_REFERENCES: &[TypedRow] = &[
    TypedRow { code: "adda" },
    TypedRow { code: "adrc" },
    TypedRow { code: "aest" },
    TypedRow { code: "auxl" },
    TypedRow { code: "avcp" },
    TypedRow { code: "cdep" },
    TypedRow { code: "cdsc" },
    TypedRow { code: "chap" },
    TypedRow { code: "deps" },
    TypedRow { code: "font" },
    TypedRow { code: "hind" },
    TypedRow { code: "hint" },
    TypedRow { code: "lyra" },
    TypedRow { code: "oref" },
    TypedRow { code: "sabt" },
    TypedRow { code: "sbas" },
    TypedRow { code: "scal" },
    TypedRow { code: "scpt" },
    TypedRow { code: "ssrc" },
    TypedRow { code: "subt" },
    TypedRow { code: "swfr" },
    TypedRow { code: "swto" },
    TypedRow { code: "tbas" },
    TypedRow { code: "thmb" },
    TypedRow { code: "tmcd" },
    TypedRow { code: "vdep" },
    TypedRow { code: "vplx" },
];

/// The TrackSelection codes a held document defines and this crate types.
pub const TYPED_TRACK_SELECTION: &[TypedRow] = &[
    TypedRow { code: "bitr" },
    TypedRow { code: "bwas" },
    TypedRow { code: "cdec" },
    TypedRow { code: "cgsc" },
    TypedRow { code: "fgsc" },
    TypedRow { code: "frar" },
    TypedRow { code: "lang" },
    TypedRow { code: "mela" },
    TypedRow { code: "mpsz" },
    TypedRow { code: "mtyp" },
    TypedRow { code: "nvws" },
    TypedRow { code: "resc" },
    TypedRow { code: "scsz" },
    TypedRow { code: "spsc" },
    TypedRow { code: "tesc" },
    TypedRow { code: "tile" },
    TypedRow { code: "vwsc" },
];

/// The KeyNamespace codes a held document defines and this crate types.
pub const TYPED_KEY_NAMESPACES: &[TypedRow] = &[
    TypedRow { code: "mdta" },
    TypedRow { code: "me4c" },
    TypedRow { code: "udta" },
    TypedRow { code: "uiso" },
];

/// The Scheme codes a held document defines and this crate types.
pub const TYPED_SCHEMES: &[TypedRow] = &[
    TypedRow { code: "aSEI" },
    TypedRow { code: "csch" },
    TypedRow { code: "essg" },
    TypedRow { code: "spkt" },
    TypedRow { code: "stvi" },
];

/// The AuxInfoType codes a held document defines and this crate types.
pub const TYPED_AUX_INFO_TYPES: &[TypedRow] =
    &[TypedRow { code: "cbcs" }, TypedRow { code: "cenc" }];

/// The ColourType codes a held document defines and this crate types.
pub const TYPED_COLOUR_TYPES: &[TypedRow] = &[
    TypedRow { code: "full" },
    TypedRow { code: "nclx" },
    TypedRow { code: "prof" },
    TypedRow { code: "rICC" },
];

/// The DataReference codes a held document defines and this crate types.
pub const TYPED_DATA_REFERENCES: &[TypedRow] = &[
    TypedRow { code: "imdt" },
    TypedRow { code: "snim" },
    TypedRow { code: "url" },
    TypedRow { code: "urn" },
];

/// The Brand codes a held document defines and this crate types.
pub const KNOWN_BRANDS: &[TypedRow] = &[
    TypedRow { code: "1pic" },
    TypedRow { code: "3ge6" },
    TypedRow { code: "3ge7" },
    TypedRow { code: "3ge9" },
    TypedRow { code: "3gf9" },
    TypedRow { code: "3gg6" },
    TypedRow { code: "3gg9" },
    TypedRow { code: "3gh9" },
    TypedRow { code: "3gm9" },
    TypedRow { code: "3gp4" },
    TypedRow { code: "3gp5" },
    TypedRow { code: "3gp6" },
    TypedRow { code: "3gp7" },
    TypedRow { code: "3gp8" },
    TypedRow { code: "3gp9" },
    TypedRow { code: "3gr6" },
    TypedRow { code: "3gr9" },
    TypedRow { code: "3gs6" },
    TypedRow { code: "3gs9" },
    TypedRow { code: "3gt8" },
    TypedRow { code: "3gt9" },
    TypedRow { code: "MA1A" },
    TypedRow { code: "MA1B" },
    TypedRow { code: "av01" },
    TypedRow { code: "avc1" },
    TypedRow { code: "avci" },
    TypedRow { code: "avcs" },
    TypedRow { code: "avif" },
    TypedRow { code: "avio" },
    TypedRow { code: "avis" },
    TypedRow { code: "ca4e" },
    TypedRow { code: "ca4m" },
    TypedRow { code: "ca4s" },
    TypedRow { code: "ceac" },
    TypedRow { code: "comp" },
    TypedRow { code: "dash" },
    TypedRow { code: "dsms" },
    TypedRow { code: "dv09" },
    TypedRow { code: "dv10" },
    TypedRow { code: "dv20" },
    TypedRow { code: "dv58" },
    TypedRow { code: "emsg" },
    TypedRow { code: "evbi" },
    TypedRow { code: "evbs" },
    TypedRow { code: "evmi" },
    TypedRow { code: "evms" },
    TypedRow { code: "heic" },
    TypedRow { code: "heim" },
    TypedRow { code: "heis" },
    TypedRow { code: "heix" },
    TypedRow { code: "hevc" },
    TypedRow { code: "hevm" },
    TypedRow { code: "hevs" },
    TypedRow { code: "hevx" },
    TypedRow { code: "hvce" },
    TypedRow { code: "hvci" },
    TypedRow { code: "hvcx" },
    TypedRow { code: "hvti" },
    TypedRow { code: "iso2" },
    TypedRow { code: "iso3" },
    TypedRow { code: "iso4" },
    TypedRow { code: "iso5" },
    TypedRow { code: "iso6" },
    TypedRow { code: "iso7" },
    TypedRow { code: "iso8" },
    TypedRow { code: "iso9" },
    TypedRow { code: "isoa" },
    TypedRow { code: "isob" },
    TypedRow { code: "isoc" },
    TypedRow { code: "isod" },
    TypedRow { code: "isom" },
    TypedRow { code: "jpeg" },
    TypedRow { code: "jpgs" },
    TypedRow { code: "lhte" },
    TypedRow { code: "lhti" },
    TypedRow { code: "lmsg" },
    TypedRow { code: "mif1" },
    TypedRow { code: "mif2" },
    TypedRow { code: "mp71" },
    TypedRow { code: "msdh" },
    TypedRow { code: "msf1" },
    TypedRow { code: "msix" },
    TypedRow { code: "nras" },
    TypedRow { code: "pred" },
    TypedRow { code: "qt" },
    TypedRow { code: "relo" },
    TypedRow { code: "risx" },
    TypedRow { code: "saie" },
    TypedRow { code: "sims" },
    TypedRow { code: "sisx" },
    TypedRow { code: "ssss" },
    TypedRow { code: "tmap" },
    TypedRow { code: "unif" },
    TypedRow { code: "vvic" },
    TypedRow { code: "vvis" },
];

/// The Handler codes a held document defines and this crate types.
pub const KNOWN_HANDLERS: &[TypedRow] = &[
    TypedRow { code: "3gsd" },
    TypedRow { code: "auxv" },
    TypedRow { code: "fdsm" },
    TypedRow { code: "hapt" },
    TypedRow { code: "hint" },
    TypedRow { code: "meta" },
    TypedRow { code: "mp7b" },
    TypedRow { code: "mp7t" },
    TypedRow { code: "mpd" },
    TypedRow { code: "mpdl" },
    TypedRow { code: "null" },
    TypedRow { code: "pict" },
    TypedRow { code: "soun" },
    TypedRow { code: "subt" },
    TypedRow { code: "text" },
    TypedRow { code: "vide" },
    TypedRow { code: "volv" },
];

/// A typed code no MP4RA table lists, with the explanation the README states.
#[derive(Debug, Clone, Copy)]
pub struct TypedNotRegistered {
    /// The kind of identifier.
    pub kind: TypedKind,
    /// The four-CC.
    pub code: &'static str,
    /// Why the crate types it anyway.
    pub reason: &'static str,
}

/// Typed codes no MP4RA table registers, each explained.
pub const TYPED_NOT_REGISTERED: &[TypedNotRegistered] = &[
    TypedNotRegistered {
        kind: TypedKind::SampleEntry,
        code: "NONE",
        reason: "QuickTime uncompressed sound entry; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntry,
        code: "lav3",
        reason: "AVS3 library sample entry the crate types; MP4RA registers the avs3 entry only",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntry,
        code: "sowt",
        reason: "QuickTime little-endian PCM entry; common in .mov files but no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntryBox,
        code: "av2c",
        reason: "AVS2 configuration box the crate types; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntryBox,
        code: "av3c",
        reason: "AVS3 configuration box the crate types; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntryBox,
        code: "dOps",
        reason: "Opus decoder configuration box (RFC 7845); no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntryBox,
        code: "ddts",
        reason: "DTS decoder configuration box; MP4RA registers the DTS sample entries but not this box",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntryBox,
        code: "la2c",
        reason: "AVS2 library configuration box the crate types; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleEntryBox,
        code: "lavc",
        reason: "AVS3 library configuration box the crate types; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::ItemProperty,
        code: "a1lx",
        reason: "AVIF layered-image-indexing property the crate types; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::ItemProperty,
        code: "a1op",
        reason: "AVIF operating-point selector property the crate types; no MP4RA table registers it",
    },
    TypedNotRegistered {
        kind: TypedKind::ItemProperty,
        code: "cmex",
        reason: "ISO/IEC 23008-12:2025 camera extrinsic matrix property; not yet in MP4RA",
    },
    TypedNotRegistered {
        kind: TypedKind::ItemProperty,
        code: "cmin",
        reason: "ISO/IEC 23008-12:2025 camera intrinsic matrix property; not yet in MP4RA",
    },
    TypedNotRegistered {
        kind: TypedKind::ItemProperty,
        code: "prdi",
        reason: "ISO/IEC 23008-12:2025 progressive derived item information property; not yet in MP4RA",
    },
    TypedNotRegistered {
        kind: TypedKind::ItemProperty,
        code: "sstr",
        reason: "ISO/IEC 23008-12:2025 single-stream property; not yet in MP4RA",
    },
    TypedNotRegistered {
        kind: TypedKind::Reference,
        code: "anot",
        reason: "ISO/IEC 23008-12:2025 region annotation reference; not yet in MP4RA",
    },
    TypedNotRegistered {
        kind: TypedKind::Reference,
        code: "drgn",
        reason: "ISO/IEC 23008-12:2025 derived-region reference; not yet in MP4RA",
    },
    TypedNotRegistered {
        kind: TypedKind::EntityGroup,
        code: "prgr",
        reason: "progressive rendering entity group; MP4RA registers the slideshow and pyramid groups but not this one",
    },
    TypedNotRegistered {
        kind: TypedKind::SampleGroup,
        code: "regm",
        reason: "ISO/IEC 23008-12:2025 region annotation sample group; not yet in MP4RA",
    },
];

/// One MP4RA table and its partition.
#[derive(Debug)]
pub struct Table {
    /// The CSV file stem under `private/specs-verified/mpeg/isobmff/mp4ra/`.
    pub csv: &'static str,
    /// The number of distinct codes in the CSV (duplicate rows are one code).
    pub rows: usize,
    /// The typed tables whose codes this table reads.
    pub typed: &'static [TypedKind],
    /// Codes the crate does not type, whose defining document is held.
    pub untyped_held: &'static [&'static str],
    /// Codes the crate does not type, whose defining document is not held.
    pub untyped_unheld: &'static [&'static str],
    /// Codes that are not ISOBMFF structures this crate parses.
    pub not_applicable: &'static [NotApplicable],
}

/// Every MP4RA table, in the order the private README lists them.
pub const TABLES: &[Table] = &[
    Table {
        csv: "aux-info-types",
        rows: 7,
        typed: &[TypedKind::AuxInfoType],
        untyped_held: &[],
        untyped_unheld: &["cbc1", "cens", "stai", "suid", "sve1"],
        not_applicable: &[],
    },
    Table {
        csv: "boxes",
        rows: 281,
        typed: &[TypedKind::Box],
        untyped_held: &[
            "3dpr", "buff", "iroi", "ldep", "mini", "mstv", "mvcg", "mvci", "mvra", "rrgn", "sdep",
            "seii", "sstl", "stmg", "stti", "svdr", "svip", "svpr", "swtc", "tibr", "tiri", "tstb",
            "vipr", "vwdi",
        ],
        untyped_unheld: &[
            "2dqr", "2dss", "Exif", "ID32", "JXL", "JXS", "UITS", "ainf", "avcn", "bidx", "bloc",
            "bmdm", "bpcc", "brob", "ccid", "cdef", "cmap", "coif", "coin", "covi", "crhd", "cstb",
            "cvru", "dihd", "dint", "dmon", "dsgd", "dstg", "evti", "fidx", "fovd", "fovi", "frpa",
            "fvsi", "grpi", "hpix", "icnu", "ihdr", "infu", "iods", "iphd", "j2kH", "j2kP", "jP",
            "jbrd", "jp2c", "jp2h", "jp2i", "jptp", "jpvi", "jpvs", "jumb", "jxlc", "jxli", "jxll",
            "jxlp", "jxpl", "loop", "lrcu", "m7hd", "mdri", "meof", "mesh", "mjhd", "mmvi", "movd",
            "ochd", "odaf", "odda", "odhd", "odhe", "odrb", "odrm", "odtt", "ohdr", "otcf", "ovly",
            "pclr", "pdat", "pfhd", "pfil", "ploc", "povd", "prfr", "pseg", "pshd", "ptle", "res",
            "resc", "resd", "rosc", "rotn", "rvif", "rwpk", "sdhd", "sdvp", "sprg", "srmb", "srmc",
            "srqr", "suep", "sumi", "surl", "trik", "uinf", "ulst", "v3sc", "vpbb", "vssn", "vunt",
            "wmpi",
        ],
        not_applicable: &[
            NotApplicable {
                code: "clip",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "crgn",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "ctab",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "fdel",
                reason: "an item-information extension carried inside infe, not a box of its own",
            },
            NotApplicable {
                code: "free",
                reason: "free-space padding with no fields; preserved opaquely by the box walker",
            },
            NotApplicable {
                code: "imap",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "imif",
                reason: "removed from ISO/IEC 14496-12:2026 (it was the IPMP information box, 2012 §8.12.3); the edition this crate implements defines no syntax for it",
            },
            NotApplicable {
                code: "ipmc",
                reason: "removed from ISO/IEC 14496-12:2026 (it was the IPMP control box, 2012 §8.12.4); the edition this crate implements defines no syntax for it",
            },
            NotApplicable {
                code: "kmat",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "load",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "matt",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "meco",
                reason: "removed from ISO/IEC 14496-12:2026 (it was the additional metadata container box, 2012 §8.11.7); the edition this crate implements defines no syntax for it",
            },
            NotApplicable {
                code: "mere",
                reason: "removed from ISO/IEC 14496-12:2026 (it was the meta box relation box, 2012 §8.11.8); the edition this crate implements defines no syntax for it",
            },
            NotApplicable {
                code: "pnot",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "skip",
                reason: "skippable free-space padding with no fields; preserved opaquely by the box walker",
            },
            NotApplicable {
                code: "uuid",
                reason: "extensible box header: the usertype selects the structure, so there is no single syntax",
            },
        ],
    },
    Table {
        csv: "boxes-qt",
        rows: 16,
        typed: &[TypedKind::QtAtom, TypedKind::Box, TypedKind::UdtaAtom],
        untyped_held: &[],
        untyped_unheld: &["csgm", "dcfD"],
        not_applicable: &[
            NotApplicable {
                code: "clip",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "crgn",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "ctab",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "imap",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "kmat",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "matt",
                reason: "reserved by the registry; no syntax is defined",
            },
            NotApplicable {
                code: "pnot",
                reason: "reserved by the registry; no syntax is defined",
            },
        ],
    },
    Table {
        csv: "boxes-udta",
        rows: 42,
        typed: &[TypedKind::UdtaAtom],
        untyped_held: &[],
        untyped_unheld: &[
            "ades", "angl", "clfn", "clid", "cmid", "cmnm", "ctlc", "date", "diap", "hpix", "manu",
            "modl", "prmd", "reel", "scen", "shot", "slno", "swre",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "brands",
        rows: 291,
        typed: &[TypedKind::Brand],
        untyped_held: &[],
        untyped_unheld: &[
            "3g2a", "3gmA", "3gtv", "3gvr", "3vra", "3vrb", "3vrm", "ARRI", "CAEP", "CDes", "J2P0",
            "J2P1", "LCAG", "M4A", "M4B", "M4P", "M4V", "MFSM", "MGSV", "MPPI", "MSNV", "MiAB",
            "MiAC", "MiAn", "MiBu", "MiCm", "MiHA", "MiHB", "MiHE", "MiPr", "ROSS", "SAV1", "SEAU",
            "SEBK", "XAVC", "XFVC", "adti", "aid3", "avde", "bbxm", "ca3a", "caaa", "caac", "cabl",
            "cafr", "cama", "camc", "caqv", "casu", "ccea", "ccff", "cdm1", "cdm4", "cevb", "cevm",
            "cfhd", "cfsd", "chd1", "chd2", "chdf", "chev", "chh1", "chhd", "cint", "clg1", "clg2",
            "clv1", "cmf2", "cmfc", "cmff", "cmfl", "cmfr", "cmfs", "cmh1", "cmh2", "cmhm", "cmhs",
            "csh1", "cud1", "cud2", "cud8", "cud9", "cuvd", "cvid", "cvvc", "cvvm", "cwvt", "da0a",
            "da0b", "da1a", "da1b", "da2a", "da2b", "da3a", "da3b", "db1p", "db2g", "db4g", "db4h",
            "dby1", "dmb1", "dts1", "dts2", "dts3", "dv1a", "dv1b", "dv2a", "dv2b", "dv3a", "dv3b",
            "dvr1", "dvt1", "dxo", "geo1", "gltf", "heoi", "hevd", "hevi", "iamf", "ifaa", "ifah",
            "ifai", "ifas", "ifau", "ifav", "ifhd", "ifhh", "ifhr", "ifhs", "ifhu", "ifhx", "ifrm",
            "ifsd", "im1i", "im1t", "im2i", "im2t", "isc2", "j2is", "j2ki", "j2ks", "jaii", "jp2",
            "jpm", "jpoi", "jpsi", "jpx", "jpxb", "jxl", "jxs", "jxsc", "jxsi", "jxss", "miaf",
            "mj2s", "mjp2", "mp21", "mp41", "mp42", "mpuf", "muxa", "niko", "nlsl", "oa2d", "oabl",
            "odcf", "ompp", "opf2", "opx2", "ovdp", "ovly", "paff", "pana", "piff", "pmff", "pnvi",
            "sdv", "senv", "siti", "sitv", "slh1", "slh2", "slh3", "sm01", "ttml", "ttwv", "uhvi",
            "uvvu", "v3mp", "v3mt", "v3nt", "v3st", "vvci", "vvoi", "vwpt", "yt4",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "checksum-types",
        rows: 2,
        typed: &[],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[
            NotApplicable {
                code: "md5",
                reason: "a digest algorithm identifier carried inside an integrity box, not a four-character box or entry type",
            },
            NotApplicable {
                code: "sha1",
                reason: "a digest algorithm identifier carried inside an integrity box, not a four-character box or entry type",
            },
        ],
    },
    Table {
        csv: "color-types",
        rows: 5,
        typed: &[TypedKind::ColourType],
        untyped_held: &[],
        untyped_unheld: &["nclc"],
        not_applicable: &[],
    },
    Table {
        csv: "data-references",
        rows: 4,
        typed: &[TypedKind::DataReference],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[],
    },
    Table {
        csv: "entity-groups",
        rows: 29,
        typed: &[TypedKind::EntityGroup],
        untyped_held: &[],
        untyped_unheld: &[
            "acgl", "amgl", "eply", "oval", "ovbg", "swpc", "swtk", "vipo", "vvcb",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "handlers",
        rows: 49,
        typed: &[TypedKind::Handler],
        untyped_held: &[],
        untyped_unheld: &[
            "GRAP", "ID32", "MPEG", "avmd", "clcp", "cpad", "crsm", "dmbd", "dtva", "dvmd", "gesm",
            "hpix", "ipdc", "ipsm", "m7sm", "mjsm", "mp21", "musi", "nnet", "nrtm", "ocsm", "odsm",
            "qd3d", "sbtl", "sdsm", "skmm", "smhr", "sprt", "strm", "tmcd", "twen", "uri",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "item-properties",
        rows: 85,
        typed: &[TypedKind::ItemProperty],
        untyped_held: &["evcC", "jpgC", "lhvC", "oinf"],
        untyped_unheld: &[
            "clev", "cloc", "cmpd", "covi", "cpal", "cpat", "disi", "fovi", "fpac", "fvsi", "iaux",
            "ienc", "iivo", "ilcp", "itai", "jaih", "jxsH", "mcrs", "mtxf", "nnrC", "ovly", "prfr",
            "rotn", "rwpk", "sbpm", "snuc", "splz", "taic", "tiep", "uncC", "v3cC", "v3tp", "vssn",
            "vutp",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "item-references",
        rows: 19,
        typed: &[TypedKind::Reference],
        untyped_held: &["evir"],
        untyped_unheld: &["auxr"],
        not_applicable: &[],
    },
    Table {
        csv: "item-types",
        rows: 27,
        typed: &[TypedKind::ItemType],
        untyped_held: &[],
        untyped_unheld: &["auxi", "j2k1", "jai0", "jxs1", "nnr1", "unci"],
        not_applicable: &[],
    },
    Table {
        csv: "key-namespaces",
        rows: 4,
        typed: &[TypedKind::KeyNamespace],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[],
    },
    Table {
        csv: "knownduplicates",
        rows: 0,
        typed: &[],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[],
    },
    Table {
        csv: "multiview-attributes",
        rows: 14,
        typed: &[TypedKind::TrackSelection, TypedKind::Box],
        untyped_held: &[
            "ecam", "elvi", "geom", "icam", "ilvi", "levl", "plvi", "prfl", "rtvi", "spvi",
        ],
        untyped_unheld: &[],
        not_applicable: &[],
    },
    Table {
        csv: "oti",
        rows: 55,
        typed: &[],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[
            NotApplicable {
                code: "00",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "01",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "02",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "03",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "04",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "05",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "06",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "07",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "08",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "09",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "0A",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "20",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "21",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "22",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "23",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "40",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "60",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "61",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "62",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "63",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "64",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "65",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "66",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "67",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "68",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "69",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "6A",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "6B",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "6C",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "6D",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "6E",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A0",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A1",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A2",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A3",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A4",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A5",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A6",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A7",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A8",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "A9",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "AA",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "AB",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "AC",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "AD",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "AE",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "AF",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "B0",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "B1",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "B2",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "B3",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "C0 - E0",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "E1",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "E2 - FE",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "FF",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-4 objectTypeIndication values, which no ISOBMFF box carries",
            },
        ],
    },
    Table {
        csv: "sample-entries",
        rows: 191,
        typed: &[TypedKind::SampleEntry],
        untyped_held: &[
            "a3d1", "a3d2", "a3d3", "a3d4", "dav1", "dts+", "dts-", "dva1", "dvav", "enc3", "encf",
            "encm", "encp", "encs", "enct", "encu", "evte", "hev2", "hev3", "hvc2", "hvc3", "hvt1",
            "hvt2", "hvt3", "icp3", "icpa", "icph", "icpm", "icpp", "icps", "icpt", "icpv", "lhe1",
            "lht1", "lhv1", "lvc1", "mjpg", "mvc1", "mvc2", "mvc3", "mvc4", "mvd1", "mvd2", "mvd3",
            "mvd4", "resa", "resv", "sbtt", "svc1", "svc2", "svcM", "vvs1",
        ],
        untyped_unheld: &[
            "2dcc", "6vpt", "FFV1", "STGS", "a3ds", "ahap", "alac", "apac", "apv1", "av3a", "avst",
            "camm", "cavs", "cndm", "depi", "dfce", "dipi", "dra1", "drac", "dtsx", "dtsy", "dyol",
            "dyvm", "dyvp", "ec+3", "evc1", "evm1", "evs1", "evs2", "fpcm", "g719", "g726", "iamf",
            "invo", "invp", "ipcm", "ixse", "j2ki", "jaim", "jxsm", "m2ts", "m4ae", "mjp2", "mlix",
            "mlpa", "mp4s", "ocpc", "oksd", "rcvp", "rtmd", "rv60", "rvp2", "savp", "scl6", "sevc",
            "sqcp", "ssmv", "stmd", "tc64", "ttsl", "tx3g", "uncv", "unid", "v3a1", "v3ag", "v3c1",
            "v3cb", "v3cg", "v3e1", "v3eg", "v3t1", "vc-1", "vp08", "vqem", "vqme", "vrsp", "vvcN",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "sample-entries-boxes",
        rows: 100,
        typed: &[
            TypedKind::SampleEntryBox,
            TypedKind::ItemProperty,
            TypedKind::Box,
        ],
        untyped_held: &[
            "3sib", "a3dC", "auxi", "dhec", "dvwC", "ecam", "hvtC", "icam", "jpgC", "lhvC", "lvcC",
            "m4ds", "mvcC", "mvcP", "mvdC", "qlif", "seib", "silb", "svcC", "svcP", "svmC", "vsib",
            "vvnC", "vwid",
        ],
        untyped_unheld: &[
            "6vpC", "CoLL", "SmDm", "acxd", "almo", "apvC", "ccfg", "clev", "cloc", "cmpd", "cpal",
            "cpat", "cuvv", "disi", "dmlp", "evcC", "evsC", "fiel", "fpac", "jaiC", "leqi", "maeI",
            "maeM", "mhaD", "mhaP", "pcmC", "pdc1", "pdi1", "prmr", "savC", "sbpm", "snuc", "splz",
            "taic", "udc1", "udc2", "udex", "udi1", "udi2", "ueqc", "ueqi", "uncC", "v3cC", "v3tC",
            "vexu", "vlab",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "sample-entries-qt",
        rows: 59,
        typed: &[TypedKind::SampleEntry],
        untyped_held: &[],
        untyped_unheld: &[
            "CFHD", "DV10", "DVOO", "DVOR", "DVTV", "DVVT", "HD10", "M105", "Shr0", "Shr1", "Shr2",
            "Shr3", "Shr4", "drac",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "sample-groups",
        rows: 67,
        typed: &[TypedKind::SampleGroup],
        untyped_held: &[],
        untyped_unheld: &[
            "2dsr", "dcfi", "ilce", "lbli", "minp", "nfas", "nfcs", "oinf", "opth", "pprr", "pss1",
            "rror", "seam", "seig", "spid", "spli", "spor", "sulm", "tmsh", "trif", "tsas", "vaps",
            "vopi",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "schemes",
        rows: 24,
        typed: &[TypedKind::Scheme],
        untyped_held: &[],
        untyped_unheld: &[
            "cbc1", "cbcs", "cenc", "cens", "cpcm", "cva1", "cva2", "cvar", "ecov", "erc2", "ercm",
            "erpv", "fodv", "meov", "modv", "odkm", "podv", "sve1", "vvvc",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "stream-types",
        rows: 15,
        typed: &[],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[
            NotApplicable {
                code: "00",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "01",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "02",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "03",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "04",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "05",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "06",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "07",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "08",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "09",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "0A",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "0B",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "0C",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "0D",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
            NotApplicable {
                code: "20- 3F",
                reason: "not an ISOBMFF four-character code: the table lists MPEG-2 stream_type values, which no ISOBMFF box carries",
            },
        ],
    },
    Table {
        csv: "textualcontent",
        rows: 25,
        typed: &[],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[
            NotApplicable {
                code: " or",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: " to",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: ".mp4",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "1111",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "3gpp",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "Name",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "PG13",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "VXYZ",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "also",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "atom",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "file",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "font",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "heif",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "hint",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "name",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "size",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "star",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "subs",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "sync",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "true",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "ttml",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "type",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "walk",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "warp",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
            NotApplicable {
                code: "wrap",
                reason: "not a four-character code: the table is a prose list of MIME/sentence checks",
            },
        ],
    },
    Table {
        csv: "track-groups",
        rows: 10,
        typed: &[TypedKind::TrackGroup],
        untyped_held: &[],
        untyped_unheld: &[
            "2dsr", "alte", "cstg", "potg", "scvg", "snut", "stvg", "vtcg",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "track-references",
        rows: 44,
        typed: &[TypedKind::TrackReference],
        untyped_held: &[],
        untyped_unheld: &[
            "cdtg", "dpnd", "evcr", "ipir", "mixn", "mpod", "recr", "rndr", "shsc", "subp", "supm",
            "sync", "v3cs", "v3ct", "v3va", "v3vg", "v3vo", "vatt", "vref", "vreg", "vvcN",
        ],
        not_applicable: &[],
    },
    Table {
        csv: "track-references-qt",
        rows: 5,
        typed: &[TypedKind::TrackReference],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[],
    },
    Table {
        csv: "track-selection",
        rows: 18,
        typed: &[TypedKind::TrackSelection],
        untyped_held: &[],
        untyped_unheld: &["swtk"],
        not_applicable: &[],
    },
    Table {
        csv: "uncv-profiles",
        rows: 17,
        typed: &[],
        untyped_held: &[],
        untyped_unheld: &[],
        not_applicable: &[
            NotApplicable {
                code: "2vuy",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "abgr",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "i420",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "nv12",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "nv21",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "rgb3",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "rgba",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "v210",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "v216",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "v308",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "v408",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "v410",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "vyuy",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "y210",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "yuv1",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "yuv2",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
            NotApplicable {
                code: "yvyu",
                reason: "not a four-character code: the table lists profile values carried inside an uncompressed-video configuration record, not a box or entry type",
            },
        ],
    },
    Table {
        csv: "unlisted",
        rows: 61,
        typed: &[
            TypedKind::Box,
            TypedKind::SampleEntry,
            TypedKind::SampleEntryBox,
            TypedKind::QtAtom,
            TypedKind::KeyNamespace,
        ],
        untyped_held: &["3dpr", "rtpo", "sroc", "tOD"],
        untyped_unheld: &[
            "ctim", "isoX", "payl", "sttg", "vlab", "vsid", "vtta", "vttc", "vtte",
        ],
        not_applicable: &[
            NotApplicable {
                code: "3gLZ",
                reason: "a registry pattern for brand codes, not a literal four-CC",
            },
            NotApplicable {
                code: "?vc?",
                reason: "a registry pattern for sample-entry codes, not a literal four-CC",
            },
            NotApplicable {
                code: "abcd",
                reason: "a registry example, not a real code",
            },
            NotApplicable {
                code: "encX",
                reason: "a registry template for encrypted entries, not a literal four-CC",
            },
            NotApplicable {
                code: "qvXt",
                reason: "a registry example brand, not a real code",
            },
            NotApplicable {
                code: "stuf",
                reason: "a registry example metadata space, not a real code",
            },
        ],
    },
];
