//! The seam between the `isobmff` box layer and this crate's IR: the error
//! conversion must preserve every variant and message, and the two track-id
//! newtypes must be interchangeable without changing what a golden hash or a
//! manifest renders.

use broadcast_common::error::{
    BufferTooShort, FieldOverflow, InvalidField, InvalidValue, OutputBufferTooSmall,
    UnsupportedVersion,
};

fn buffer_too_short() -> isobmff::Error {
    isobmff::Error::BufferTooShort(BufferTooShort {
        need: 8,
        have: 4,
        what: "test",
    })
}

#[test]
fn every_isobmff_error_maps_to_its_transmux_variant() {
    use isobmff::Error as I;
    use transmux::Error as T;

    // Shared kinds: the same variant, and the same Display text.
    type Case = (I, fn(&T) -> bool, &'static str);
    let cases: Vec<Case> = vec![
        (
            buffer_too_short(),
            |e| matches!(e, T::BufferTooShort(_)),
            "buffer too short: need 8 bytes, have 4 (while parsing test)",
        ),
        (
            I::LargesizeBufferTooShort { need: 8, have: 2 },
            |e| matches!(e, T::LargesizeBufferTooShort { need: 8, have: 2 }),
            "largesize indicated but buffer too short: need 8, have 2",
        ),
        (
            I::UuidBufferTooShort { need: 16, have: 3 },
            |e| matches!(e, T::UuidBufferTooShort { need: 16, have: 3 }),
            "uuid box indicated but buffer too short: need 16, have 3",
        ),
        (
            I::BoxSizeUnderflow {
                size: 4,
                header_size: 8,
            },
            |e| {
                matches!(
                    e,
                    T::BoxSizeUnderflow {
                        size: 4,
                        header_size: 8
                    }
                )
            },
            "box size 4 is smaller than header (8 bytes)",
        ),
        (
            I::OutputBufferTooSmall(OutputBufferTooSmall { need: 8, have: 4 }),
            |e| matches!(e, T::OutputBufferTooSmall(_)),
            "output buffer too small: need 8, have 4",
        ),
        (
            I::InvalidValue(InvalidValue {
                field: "f",
                value: 9,
                reason: "bad",
            }),
            |e| matches!(e, T::InvalidValue(_)),
            "field f value 9 invalid: bad",
        ),
        (
            I::InvalidField(InvalidField {
                field: "f",
                reason: "bad",
            }),
            |e| matches!(e, T::InvalidField(_)),
            "invalid field f: bad",
        ),
        (
            I::UnexpectedBox { expected: "moof" },
            |e| matches!(e, T::UnexpectedBox { expected: "moof" }),
            "unexpected box: expected moof",
        ),
        (
            I::InvalidInput("nope"),
            |e| matches!(e, T::InvalidInput("nope")),
            "invalid input: nope",
        ),
        (
            I::ZeroWidthTrunBudgetExceeded { limit: 5 },
            |e| matches!(e, T::ZeroWidthTrunBudgetExceeded { limit: 5 }),
            "trun sample_count with no per-sample fields exceeds MAX_ZERO_WIDTH_TRUN_SAMPLES \
             (5, aggregate per moof)",
        ),
        (
            I::SampleDataBudgetExceeded { limit: 9 },
            |e| matches!(e, T::SampleDataBudgetExceeded { limit: 9 }),
            "sample data budget exceeded: resolved sample bytes exceed the 9-byte allowance \
             (overlapping chunks, runs or fragments?)",
        ),
        (
            I::LimitExceeded {
                what: "samples per track",
                limit: 5,
            },
            |e| {
                matches!(
                    e,
                    T::LimitExceeded {
                        what: "samples per track",
                        limit: 5
                    }
                )
            },
            "samples per track exceeds the configured limit of 5",
        ),
        (
            I::FragmentTooLarge { limit: 7 },
            |e| matches!(e, T::FragmentTooLarge { limit: 7 }),
            "fragment exceeds the configured limit of 7 bytes",
        ),
        (
            I::FieldOverflow(FieldOverflow {
                field: "f",
                value: 9,
                max: 3,
            }),
            |e| matches!(e, T::FieldOverflow(_)),
            "f = 9 exceeds the field maximum 3",
        ),
        (
            I::UnsupportedFeature("seig"),
            |e| matches!(e, T::UnsupportedFeature("seig")),
            "unsupported feature: seig",
        ),
    ];
    // A new `isobmff::Error` variant needs a case here and an arm in the
    // `From` impl; the fallback check below catches it if only the arm is
    // forgotten.
    const FALLBACK: &str = "unsupported feature: unrecognised ISO BMFF box error";
    for (isobmff_err, matches_transmux, text) in cases {
        assert_eq!(isobmff_err.to_string(), text);
        let mapped = transmux::Error::from(isobmff_err);
        assert!(matches_transmux(&mapped), "mapped to {mapped:?}");
        assert_ne!(
            mapped.to_string(),
            FALLBACK,
            "{text}: converted to the From fallback; add an arm in transmux::Error::from(isobmff::Error)"
        );
    }

    // The `emsg` parser's own variants (and its version check) have always
    // surfaced here as `EmsgSerialize`; they still do.
    let emsg_cases = [
        I::UnsupportedVersion(UnsupportedVersion {
            what: "emsg",
            found: 2,
        }),
        I::NotEmsg { found: *b"free" },
        I::InvalidSize {
            size: 7,
            reason: "short",
        },
        I::InvalidString {
            field: "value",
            reason: "unterminated",
        },
    ];
    for isobmff_err in emsg_cases {
        let mapped = transmux::Error::from(isobmff_err);
        assert!(
            matches!(mapped, T::EmsgSerialize(_)),
            "emsg variant mapped to {mapped:?}"
        );
    }
}

#[test]
fn track_ids_print_and_serialise_alike() {
    use isobmff::TrackId as Iso;
    use transmux::TrackId as Ir;

    for raw in [0u32, 1, 7, u32::MAX] {
        let iso = Iso::new(raw);
        let ir = Ir::new(raw);
        assert_eq!(format!("{iso:?}"), format!("{ir:?}"));
        assert_eq!(format!("{iso}"), format!("{ir}"));
        assert_eq!(iso.value(), ir.value());
        assert_eq!(Iso::from(ir), iso);
        assert_eq!(Ir::from(iso), ir);
    }
    assert_eq!(format!("{:?}", Some(Iso::new(3))), "Some(3)");

    #[cfg(feature = "serde")]
    {
        assert_eq!(serde_json::to_string(&Iso::new(4)).unwrap(), "4");
        assert_eq!(
            serde_json::to_string(&Ir::new(4)).unwrap(),
            serde_json::to_string(&Iso::new(4)).unwrap()
        );
    }
}

/// `Fmp4Demux` scans the file's `mdat`s exactly once however many
/// fragments/trafs it holds (was once per traf: O(fragments x boxes)).
#[cfg(feature = "scan-counters")]
#[test]
fn demux_scans_mdats_once_per_file() {
    use broadcast_common::Unpackage;
    let path = test_fixtures::path("mp4/cenc-layouts/clear-multitrun-omit-implicit.mp4");
    let bytes = std::fs::read(path).expect("fixture");
    let before = isobmff::frag_offsets::mdat_scans_on_thread();
    let media = transmux::Fmp4Demux::new()
        .unpackage(bytes.as_slice())
        .expect("demux");
    assert!(media.tracks.iter().map(|t| t.samples.len()).sum::<usize>() > 1);
    assert_eq!(
        isobmff::frag_offsets::mdat_scans_on_thread() - before,
        1,
        "one mdat scan per file"
    );
}
