//! Per-crate data for the `no_dom` guard. The scope is exactly these crates
//! (the ones that read or write XML, all through `quick_xml`); add a line for
//! a new crate that touches XML. Every allowlist entry says why the item is
//! not an XML tree or an escaper.

use crate::no_dom::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-common",
        allowed_types: &[
            (
                "Elem",
                "the bounded XML reader's element node (namespace, local name, attributes, kept children, text); the one tree the shared reader builds, depth- and count-limited",
            ),
            (
                "XmlError",
                "the reader's rejection reason (invalid UTF-8, not well-formed, foreign root, exceeded bound), not a node",
            ),
            (
                "XmlCharError",
                "the pull parsers' rejection reason (an outside-the-Char character, an undefined entity, an undeclared prefix, a reader fault), not a node",
            ),
            (
                "XmlLimits",
                "the hostile-input ceilings (bytes, depth, elements, attribute size/count) the reader enforces, not a node",
            ),
            ("XmlLimit", "which ceiling tripped, not a node"),
            (
                "XmlBudget",
                "the running tally of one document against its ceilings, not a node",
            ),
            (
                "XmlWriter",
                "an indenting quick-xml writer alias shared by the manifest writers, never read back into a tree",
            ),
        ],
        allowed_fns: &[
            (
                "is_xml_char",
                "XML 1.0 section 2.2 Char predicate used to validate decoded text, not an escaper",
            ),
            (
                "char_data",
                "decodes one quick-xml event's character data (text, CDATA, resolved references) through the Char predicate, not a hand-rolled unescaper",
            ),
        ],
        needs_xml_module: true,
    },
    Crate {
        name: "dvb-mabr",
        allowed_types: &[],
        allowed_fns: &[
            (
                "to_xml",
                "public serialization entry point (writes through quick_xml::Writer), not an escaper",
            ),
            ("write_xml", "element writers that emit quick-xml events"),
            (
                "is_xml_char",
                "XML 1.0 section 2.2 Char predicate used to validate decoded text, not an escaper",
            ),
            ("xml_error", "maps a quick-xml error to Error::XmlParse"),
        ],
        needs_xml_module: true,
    },
    Crate {
        name: "media-doctor",
        allowed_types: &[],
        allowed_fns: &[],
        // its DASH check calls `dash_runtime::mpd::Mpd::parse`; the type/entity/markup rules still apply
        needs_xml_module: false,
    },
    Crate {
        name: "multimux",
        allowed_types: &[],
        allowed_fns: &[],
        needs_xml_module: true,
    },
    Crate {
        name: "broadcast-smooth",
        allowed_types: &[],
        allowed_fns: &[],
        // reads through `broadcast_common::xml`; the type/entity/markup rules still apply
        needs_xml_module: false,
    },
    Crate {
        name: "transmux",
        allowed_types: &[(
            "XmlSubtitleSampleEntry",
            "ISO BMFF stpp sample entry (XML subtitle track) box, unrelated to XML parsing",
        )],
        allowed_fns: &[
            (
                "xml_error",
                "maps a quick-xml error to the parser error enum",
            ),
            (
                "xml_message",
                "builds a positioned parser error from a validation message",
            ),
            (
                "write_service_description",
                "writes <ServiceDescription>/<UTCTiming> as quick-xml events",
            ),
            (
                "is_xml_char",
                "XML 1.0 section 2.2 Char predicate used to validate decoded text, not an escaper",
            ),
        ],
        needs_xml_module: true,
    },
    Crate {
        name: "ttml-subtitle",
        allowed_types: &[
            (
                "AudioElement",
                "TTML2 typed model struct for the <audio> element (named fields, not a generic node)",
            ),
            (
                "BodyElement",
                "TTML2 typed model struct for the <body> element (named fields, not a generic node)",
            ),
            (
                "BrElement",
                "TTML2 typed model struct for the <br> element (named fields, not a generic node)",
            ),
            (
                "ChunkElement",
                "TTML2 typed model struct for the <chunk> element (named fields, not a generic node)",
            ),
            (
                "DataElement",
                "TTML2 typed model struct for the <data> element (named fields, not a generic node)",
            ),
            (
                "DivElement",
                "TTML2 typed model struct for the <div> element (named fields, not a generic node)",
            ),
            (
                "EbuttmElement",
                "TTML2 typed model struct for the <ebuttm:documentMetadata> element (named fields, not a generic node)",
            ),
            (
                "EbuttmTextElement",
                "TTML2 typed model struct for the <ebuttm:conformsToStandard> element (named fields, not a generic node)",
            ),
            (
                "FontElement",
                "TTML2 typed model struct for the <font> element (named fields, not a generic node)",
            ),
            (
                "HeadElement",
                "TTML2 typed model struct for the <head> element (named fields, not a generic node)",
            ),
            (
                "ImageElement",
                "TTML2 typed model struct for the <image> element (named fields, not a generic node)",
            ),
            (
                "InitialElement",
                "TTML2 typed model struct for the <initial> element (named fields, not a generic node)",
            ),
            (
                "IttmAltTextElement",
                "TTML2 typed model struct for the <ittm:altText> element (named fields, not a generic node)",
            ),
            (
                "LayoutElement",
                "TTML2 typed model struct for the <layout> element (named fields, not a generic node)",
            ),
            (
                "MetadataElement",
                "TTML2 typed model struct for the <metadata> element (named fields, not a generic node)",
            ),
            (
                "PElement",
                "TTML2 typed model struct for the <p> element (named fields, not a generic node)",
            ),
            (
                "RegionElement",
                "TTML2 typed model struct for the <region> element (named fields, not a generic node)",
            ),
            (
                "ResourcesElement",
                "TTML2 typed model struct for the <resources> element (named fields, not a generic node)",
            ),
            (
                "SetElement",
                "TTML2 typed model struct for the <set> element (named fields, not a generic node)",
            ),
            (
                "SourceElement",
                "TTML2 typed model struct for the <source> element (named fields, not a generic node)",
            ),
            (
                "SpanElement",
                "TTML2 typed model struct for the <span> element (named fields, not a generic node)",
            ),
            (
                "StyleElement",
                "TTML2 typed model struct for the <style> element (named fields, not a generic node)",
            ),
            (
                "StylingElement",
                "TTML2 typed model struct for the <styling> element (named fields, not a generic node)",
            ),
            (
                "TtElement",
                "TTML2 typed model struct for the <tt> element (named fields, not a generic node)",
            ),
            (
                "TtmAgentElement",
                "TTML2 typed model struct for the <ttm:agent> element (named fields, not a generic node)",
            ),
            (
                "TtmItemElement",
                "TTML2 typed model struct for the <ttm:item> element (named fields, not a generic node)",
            ),
            (
                "TtmNameElement",
                "TTML2 typed model struct for the <ttm:name> element (named fields, not a generic node)",
            ),
            (
                "TtmTextElement",
                "TTML2 typed model struct for the <ttm:title/desc/copyright> element (named fields, not a generic node)",
            ),
            (
                "UnknownElement",
                "lossless capture of ONE foreign element the TTML model does not type (TTML2 section 7.2/7.3), part of the public model, depth-bounded; not a document tree",
            ),
            (
                "UnknownNode",
                "child of UnknownElement: Element or Text, the public model for foreign content; not a document tree",
            ),
            (
                "WalkNode",
                "explicit-stack frame for the iterative walk over the typed MetadataChild model when collecting namespaces; not an XML node",
            ),
            (
                "XmlSpace",
                "the xml:space attribute value (default|preserve), an enum of attribute values",
            ),
            (
                "XmlDeclaration",
                "the parsed <?xml ...?> declaration (version, encoding) of the typed document",
            ),
        ],
        allowed_fns: &[
            (
                "to_xml",
                "public serialization entry point (writes through quick_xml::Writer), not an escaper",
            ),
            (
                "is_xml_char",
                "XML 1.0 section 2.2 Char predicate used to validate decoded text, not an escaper",
            ),
            (
                "parse_xml_space",
                "parses the xml:space attribute value into the typed XmlSpace",
            ),
            (
                "serialize_xml_space",
                "writes the xml:space attribute via quick-xml",
            ),
            (
                "serialize_xml_base",
                "writes the xml:base attribute via quick-xml",
            ),
        ],
        needs_xml_module: true,
    },
];
