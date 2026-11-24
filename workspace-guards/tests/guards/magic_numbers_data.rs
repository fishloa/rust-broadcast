//! Per-crate data for the `magic_numbers` guard (#4051). Every crate in the
//! table carries its reviewed allowlist, one entry per exact trimmed source line
//! with its reason; a finding with no entry fails the guard, and so does an
//! entry that no longer matches. `EXEMPT` names the crates with no production
//! wire code to police. Add a line for a new crate.

use crate::magic_numbers::{Crate, Exemption};

/// Why a workspace member has no production wire code to police.
pub const EXEMPT: &[Exemption] = &[
    (
        "workspace-guards",
        "unpublished: the guards' own test bodies and fixtures are not production wire code",
    ),
    (
        "test-fixtures",
        "unpublished test-helper crate: fixture path tables, not production wire code",
    ),
    (
        "test-alloc",
        "unpublished test-helper crate: the counting allocator's budget constants serve tests only",
    ),
    (
        "test-harness",
        "unpublished test-helper crate: test harness scaffolding, not production wire code",
    ),
];

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-common",
        allow: &[
            (
                "src/clock33.rs",
                "pub const WRAP_33BIT_HALF: u64 = WRAP_33BIT / 2;",
                "\"half\" names the wrap-classification threshold, not a count (ISO/IEC 13818-1 §2.4.3.7); public API constant name; renaming would break the published API",
            ),
            (
                "src/clock33.rs",
                "pub const ZERO: Self = Self(0);",
                "associated-const idiom (cf. Duration::ZERO); public API, cannot be renamed; public API constant name; renaming would break the published API",
            ),
            (
                "src/stage.rs",
                "pub const ZERO: Timestamp = Timestamp(0);",
                "associated-const idiom; public API, cannot be renamed; public API constant name; renaming would break the published API",
            ),
        ],
    },
    Crate {
        name: "broadcast-auth",
        allow: &[],
    },
    Crate {
        name: "dvb-si",
        allow: &[
            (
                "src/descriptors/aac.rs",
                "pub const RESERVED_ZERO_FUTURE_USE_DEFAULT: ReservedZero<RESERVED_FUTURE_USE_BITS> =",
                "names the EN 300 468 field `reserved_zero_future_use`, not a value; the spec field is reserved_zero_future_use, not a number",
            ),
            (
                "src/descriptors/content.rs",
                "(0x0, 0x0..=0xF) => \"undefined content\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x0) => \"movie/drama (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x1) => \"detective/thriller\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x2) => \"adventure/western/war\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x3) => \"science fiction/fantasy/horror\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x4) => \"comedy\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x5) => \"soap/melodrama/folkloric\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x6) => \"romance\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x7) => \"serious/classical/religious/historical movie/drama\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x8) => \"adult movie/drama\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0x9..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x1, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0x0) => \"news/current affairs (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0x1) => \"news/weather report\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0x2) => \"news magazine\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0x3) => \"documentary\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0x4) => \"discussion/interview/debate\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0x5..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x2, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x3, 0x0) => \"show/game show (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x3, 0x1) => \"game show/quiz/contest\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x3, 0x2) => \"variety show\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x3, 0x3) => \"talk show\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x3, 0x4..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x3, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x0) => \"sports (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x1) => \"special events (Olympic Games, World Cup, etc.)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x2) => \"sports magazines\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x3) => \"football/soccer\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x4) => \"tennis/squash\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x5) => \"team sports (excluding football)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x6) => \"athletics\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x7) => \"motor sport\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x8) => \"water sport\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0x9) => \"winter sports\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0xA) => \"equestrian\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0xB) => \"martial sports\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0xC..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x4, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x0) => \"children's/youth programmes (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x1) => \"pre-school children's programmes\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x2) => \"entertainment programmes for 6 to 14\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x3) => \"entertainment programmes for 10 to 16\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x4) => \"informational/educational/school programmes\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x5) => \"cartoons/puppets\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0x6..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x5, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x0) => \"music/ballet/dance (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x1) => \"rock/pop\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x2) => \"serious music/classical music\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x3) => \"folk/traditional music\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x4) => \"jazz\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x5) => \"musical/opera\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x6) => \"ballet\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0x7..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x6, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x0) => \"arts/culture (without music, general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x1) => \"performing arts\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x2) => \"fine arts\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x3) => \"religion\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x4) => \"popular culture/traditional arts\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x5) => \"literature\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x6) => \"film/cinema\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x7) => \"experimental film/video\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x8) => \"broadcasting/press\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0x9) => \"new media\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0xA) => \"arts/culture magazines\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0xB) => \"fashion\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0xC..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x7, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x8, 0x0) => \"social/political issues/economics (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x8, 0x1) => \"magazines/reports/documentary\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x8, 0x2) => \"economics/social advisory\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x8, 0x3) => \"remarkable people\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x8, 0x4..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x8, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x0) => \"education/science/factual topics (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x1) => \"nature/animals/environment\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x2) => \"technology/natural sciences\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x3) => \"medicine/physiology/psychology\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x4) => \"foreign countries/expeditions\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x5) => \"social/spiritual sciences\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x6) => \"further education\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x7) => \"languages\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0x8..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0x9, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x0) => \"leisure hobbies (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x1) => \"tourism/travel\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x2) => \"handicraft\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x3) => \"motoring\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x4) => \"fitness and health\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x5) => \"cooking\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x6) => \"advertisement/shopping\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x7) => \"gardening\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0x8..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xA, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x0) => \"original language\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x1) => \"black and white\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x2) => \"unpublished\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x3) => \"live broadcast\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x4) => \"plano-stereoscopic\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x5) => \"local or regional\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0x6..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xB, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xC, 0x0) => \"adult (general)\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xC, 0x1..=0xE) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xC, 0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xD..=0xE, 0x0..=0xF) => \"reserved\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/content.rs",
                "(0xF, 0x0..=0xF) => \"user defined\",",
                "EN 300 468 Table 29 content-genre code mapping; EN 300 468 Table 29 genre mapping arm (spec-code table, not a magic number)",
            ),
            (
                "src/descriptors/int/mod.rs",
                "@owned 0x00..=0x3F,",
                "declare_scoped_namespace owned tag-range row; declare_scoped_namespace! row: INT-owned descriptor tag range, not a magic number",
            ),
            (
                "src/descriptors/mpeg2_ext_codecs.rs",
                "(1, 2) | (2, 3) => {}",
                "match arm mapping lang_len_idc values to fixed language-code lengths; Table 2-150 lang_len_idc -> language-code length mapping",
            ),
            (
                "src/descriptors/unt/mod.rs",
                "@owned 0x00..=0x3F,",
                "declare_scoped_namespace owned tag-range row; declare_scoped_namespace! row: UNT-owned descriptor tag range, not a magic number",
            ),
        ],
    },
    Crate {
        name: "dvb-t2mi",
        allow: &[],
    },
    Crate {
        name: "dvb-bbframe",
        allow: &[],
    },
    Crate {
        name: "dvb-conformance",
        allow: &[],
    },
    Crate {
        name: "dvb-tools",
        allow: &[],
    },
    Crate {
        name: "media-doctor",
        allow: &[],
    },
    Crate {
        name: "multimux",
        allow: &[],
    },
    Crate {
        name: "multimux-cli",
        allow: &[],
    },
    Crate {
        name: "scte35-splice",
        allow: &[],
    },
    Crate {
        name: "dvb-stream",
        allow: &[],
    },
    Crate {
        name: "mpeg-pes",
        allow: &[],
    },
    Crate {
        name: "dvb-subtitle",
        allow: &[],
    },
    Crate {
        name: "mpeg-ps",
        allow: &[],
    },
    Crate {
        name: "scte104",
        allow: &[],
    },
    Crate {
        name: "cc-data",
        allow: &[],
    },
    Crate {
        name: "dvb-ci",
        allow: &[],
    },
    Crate {
        name: "st291",
        allow: &[],
    },
    Crate {
        name: "ule",
        allow: &[],
    },
    Crate {
        name: "dvb-simulcrypt",
        allow: &[],
    },
    Crate {
        name: "mp4-emsg",
        allow: &[],
    },
    Crate {
        name: "dvb-vbi",
        allow: &[],
    },
    Crate {
        name: "rmt-flute",
        allow: &[],
    },
    Crate {
        name: "dvb-ci-runtime",
        allow: &[],
    },
    Crate {
        name: "timed-metadata",
        allow: &[],
    },
    Crate {
        name: "mpeg-ts",
        allow: &[(
            "src/ts.rs",
            "pub const ZERO: Self = Self(0);",
            "associated-const idiom; public API, cannot be renamed; public API constant name; renaming would break the published API",
        )],
    },
    Crate {
        name: "ts-fix",
        allow: &[],
    },
    Crate {
        name: "transmux",
        allow: &[
            (
                "src/annexb.rs",
                "pub const NAL_LENGTH_SIZE_MINUS_ONE: u8 = (NAL_LENGTH_SIZE - 1) as u8;",
                "public API naming the ISO/IEC 14496-15:2017 §5.3.3.1.3 field lengthSizeMinusOne; renaming it is a breaking API change; spec field name, value derived",
            ),
            (
                "src/dovi.rs",
                "pub const DOVI_RESERVED_ZERO: [u8; DOVI_RESERVED_LEN] = [0; DOVI_RESERVED_LEN];",
                "public API constant naming the spec's all-reserved-bits-zero value, not a magic number; renaming it is a breaking API change; reserved zero fill, not a magic number",
            ),
        ],
    },
    Crate {
        name: "isobmff",
        allow: &[
            (
                "src/box_support.rs",
                "pub const ZERO_WIDTH_PREALLOC: usize = 1024;",
                "names zero-width entries, not the number; referenced by test code; preallocation bound, role-named",
            ),
            (
                "src/movie_fragment.rs",
                "pub const MAX_ZERO_WIDTH_TRUN_SAMPLES: usize = 1 << 20;",
                "existing PUBLIC API constant (name predates this refactor; renaming would be a breaking change), not a magic literal; public cap on zero-width trun samples",
            ),
        ],
    },
    Crate {
        name: "rtsp-runtime",
        allow: &[],
    },
    Crate {
        name: "rtmp-runtime",
        allow: &[(
            "src/ids.rs",
            "pub const ZERO: Self = Self(Duration::ZERO);",
            "associated-const idiom; public API, cannot be renamed; public API constant `ZERO`; renaming would break the public API",
        )],
    },
    Crate {
        name: "srt-runtime",
        allow: &[
            (
                "src/packet/number.rs",
                "pub const ZERO: AckNumber = AckNumber(0);",
                "associated-const idiom; public API, cannot be renamed; public API constant `ZERO`; renaming would break the public API",
            ),
            (
                "src/packet/number.rs",
                "pub const ZERO: SrtTimestamp = SrtTimestamp(0);",
                "associated-const idiom; public API, cannot be renamed; public API constant `ZERO`; renaming would break the public API",
            ),
        ],
    },
    Crate {
        name: "rtp-packet",
        allow: &[
            (
                "src/rfc8285.rs",
                "pub const ONE_BYTE_PROFILE_ID: u16 = 0xBEDE;",
                "\"one-byte\" is RFC 8285's name for the header-extension form; public API, cannot be renamed; public API constant name (spec identifier); renaming would break the published API",
            ),
            (
                "src/seq.rs",
                "pub const RTP_SEQ_MOD_PLUS_ONE: u32 = RTP_SEQ_MOD + 1;",
                "names its relation to the RFC 3550 §A.1 modulus; public API, cannot be renamed; public API constant name (spec identifier); renaming would break the published API",
            ),
        ],
    },
    Crate {
        name: "st12-1",
        allow: &[],
    },
    Crate {
        name: "st337",
        allow: &[],
    },
    Crate {
        name: "rdd29",
        allow: &[],
    },
    Crate {
        name: "st377-1",
        allow: &[],
    },
    Crate {
        name: "hls-runtime",
        allow: &[],
    },
    Crate {
        name: "dash-runtime",
        allow: &[],
    },
    Crate {
        name: "media-plane",
        allow: &[],
    },
    Crate {
        name: "ttml-subtitle",
        allow: &[],
    },
    Crate {
        name: "broadcast-hls",
        allow: &[
            (
                "src/lib.rs",
                "pub const ZERO: Self = Self(0.0);",
                "associated-const idiom; public API, cannot be renamed; public API constant `ZERO`; renaming would break the public API",
            ),
            (
                "src/seq.rs",
                "pub const ZERO: MediaSequence = MediaSequence(0);",
                "associated-const idiom; public API, cannot be renamed; public API constant `ZERO`; renaming would break the public API",
            ),
            (
                "src/seq.rs",
                "pub const ZERO: PartIndex = PartIndex(0);",
                "associated-const idiom; public API, cannot be renamed; public API constant `ZERO`; renaming would break the public API",
            ),
        ],
    },
    Crate {
        name: "broadcast-dash",
        allow: &[(
            "src/descriptors.rs",
            "pub const XLINK_RESOLVE_TO_ZERO: &str = \"urn:mpeg:dash:resolve-to-zero:2013\";",
            "public API name mirrors the spec token `resolve-to-zero`; renaming would break the public API; the name is the spec URN (ISO/IEC 23009-1), not a count",
        )],
    },
    Crate {
        name: "broadcast-loudness",
        allow: &[],
    },
    Crate {
        name: "buffering-runtime",
        allow: &[],
    },
    Crate {
        name: "broadcast-client",
        allow: &[],
    },
    Crate {
        name: "dvb-csa",
        allow: &[],
    },
    Crate {
        name: "dvb-mabr",
        allow: &[],
    },
    Crate {
        name: "atsc3",
        allow: &[],
    },
    Crate {
        name: "atsc3-route",
        allow: &[],
    },
    Crate {
        name: "st2022",
        allow: &[],
    },
    Crate {
        name: "rist-runtime",
        allow: &[],
    },
    Crate {
        name: "webrtc-runtime",
        allow: &[],
    },
    Crate {
        name: "caption-convert",
        allow: &[],
    },
    Crate {
        name: "compliance-probe",
        allow: &[],
    },
    Crate {
        name: "ssai-runtime",
        allow: &[],
    },
    Crate {
        name: "playout-runtime",
        allow: &[],
    },
    Crate {
        name: "container-probe",
        allow: &[],
    },
    Crate {
        name: "broadcast-smooth",
        allow: &[],
    },
    Crate {
        name: "smooth-runtime",
        allow: &[],
    },
    Crate {
        name: "dvr-runtime",
        allow: &[],
    },
];
