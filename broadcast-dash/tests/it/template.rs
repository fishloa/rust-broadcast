//! `SegmentTemplate` identifier grammar (ISO/IEC 23009-1 5.3.9.4.4 Table 21)
//! and the inverse URL matcher, plus the template error labels.

use broadcast_dash::template::{
    MAX_FORMAT_WIDTH, TemplateError, TemplateFault, TemplateMatch, TemplateUse, format_width,
    match_url, scan, scan_initialization, scan_media, substitute,
};

fn use_of(template: &str) -> TemplateUse {
    scan(template).unwrap_or_else(|e| panic!("{template:?}: {e}"))
}

fn used(one: impl FnOnce(&mut TemplateUse)) -> TemplateUse {
    let mut u = TemplateUse::default();
    one(&mut u);
    u
}

fn matched(with: impl FnOnce(&mut TemplateMatch)) -> TemplateMatch {
    let mut m = TemplateMatch::default();
    with(&mut m);
    m
}

#[test]
fn scan_reports_every_table_21_identifier() {
    assert_eq!(
        use_of("$RepresentationID$"),
        used(|u| u.representation_id = true)
    );
    assert_eq!(use_of("$Number$"), used(|u| u.number = true));
    assert_eq!(use_of("$Time$"), used(|u| u.time = true));
    assert_eq!(use_of("$Bandwidth$"), used(|u| u.bandwidth = true));
    let all = use_of("$RepresentationID$-$Number$-$Time$-$Bandwidth$-$SubNumber$");
    assert!(all.representation_id);
    assert!(all.number);
    assert!(all.time);
    assert!(all.bandwidth);
    assert!(all.sub_number);
    assert_eq!(use_of("seg.m4s"), TemplateUse::default());
}

#[test]
fn a_dollar_escape_carries_no_identifier() {
    assert_eq!(use_of("a$$b"), TemplateUse::default());
    assert_eq!(
        substitute("a$$b-$Number$.m4s", "r", Some(7), None, 0),
        Ok("a$b-7.m4s".to_owned())
    );
}

#[test]
fn scan_rejects_an_unterminated_or_unknown_identifier() {
    assert_eq!(
        scan("chunk-$Number"),
        Err(TemplateFault::UnterminatedIdentifier)
    );
    assert_eq!(
        scan("chunk-$Number.m4s"),
        Err(TemplateFault::UnterminatedIdentifier)
    );
    assert_eq!(scan("$Foo$"), Err(TemplateFault::UnknownIdentifier));
    assert_eq!(scan("$number$"), Err(TemplateFault::UnknownIdentifier));
}

#[test]
fn a_format_tag_is_zero_flag_width_d_only() {
    assert_eq!(scan("$Number%05d$"), Ok(used(|u| u.number = true)));
    assert_eq!(scan("$Number%5d$"), Err(TemplateFault::BadFormatTag));
    assert_eq!(scan("$Number%0xd$"), Err(TemplateFault::BadFormatTag));
    assert_eq!(scan("$Number%05$"), Err(TemplateFault::BadFormatTag));
    assert_eq!(scan("$Number%05x$"), Err(TemplateFault::BadFormatTag));
    assert_eq!(
        scan("$RepresentationID%05d$"),
        Err(TemplateFault::FormatOnRepresentationId)
    );
}

#[test]
fn the_format_width_boundary_is_max_format_width() {
    assert_eq!(MAX_FORMAT_WIDTH, 20);
    let at_max = format!("$Number%0{MAX_FORMAT_WIDTH}d$");
    assert_eq!(scan(&at_max), Ok(used(|u| u.number = true)));
    let over = format!("$Number%0{}d$", MAX_FORMAT_WIDTH + 1);
    assert_eq!(scan(&over), Err(TemplateFault::WidthTooLarge));
}

#[test]
fn scan_media_needs_exactly_one_dynamic_identifier() {
    assert_eq!(scan_media("m$Number$.m4s").map(|u| u.number), Ok(true));
    assert_eq!(scan_media("m$Time$.m4s").map(|u| u.time), Ok(true));
    assert_eq!(
        scan_media("m$Number$-$Time$.m4s"),
        Err(TemplateFault::NumberAndTime)
    );
    assert_eq!(
        scan_media("m$RepresentationID$.m4s"),
        Err(TemplateFault::MissingNumberOrTime)
    );
    assert_eq!(
        scan_media("m$Number"),
        Err(TemplateFault::UnterminatedIdentifier)
    );
}

#[test]
fn scan_initialization_refuses_a_dynamic_identifier() {
    assert_eq!(
        scan_initialization("init-$Number$.mp4"),
        Err(TemplateFault::DynamicIdentifierInInitialization)
    );
    assert_eq!(
        scan_initialization("init-$Time$.mp4"),
        Err(TemplateFault::DynamicIdentifierInInitialization)
    );
    assert_eq!(
        scan_initialization("$RepresentationID$/init.mp4").map(|u| u.representation_id),
        Ok(true)
    );
}

#[test]
fn substitute_renders_a_bandwidth_and_a_clamped_width() {
    assert_eq!(
        substitute("b-$Bandwidth%07d$.m4s", "r", None, None, 128_000),
        Ok("b-0128000.m4s".to_owned())
    );
    assert_eq!(
        substitute("b-$Number%00005d$.m4s", "r", Some(12), None, 0),
        Ok("b-00012.m4s".to_owned())
    );
}

#[test]
fn format_width_zero_pads_and_never_truncates() {
    assert_eq!(format_width(7, 5), "00007");
    assert_eq!(format_width(123_456, 3), "123456");
    assert_eq!(format_width(1, 0), "1");
    assert_eq!(format_width(0, 2), "00");
    let wide = format_width(1, MAX_FORMAT_WIDTH + 9);
    assert_eq!(wide.len(), MAX_FORMAT_WIDTH);
    assert_eq!(wide, format!("{}1", "0".repeat(MAX_FORMAT_WIDTH - 1)));
}

const IDS: &[&str] = &["r1", "r10"];

#[test]
fn match_url_recovers_the_representation_and_number() {
    assert_eq!(
        match_url("$RepresentationID$/$Number$.m4s", "r10/5.m4s", IDS),
        Some(matched(|m| {
            m.representation_id = "r10".to_string();
            m.number = Some(5);
        }))
    );
    assert_eq!(
        match_url("$RepresentationID$/$Number%02d$.m4s", "r1/05.m4s", IDS)
            .map(|m| (m.representation_id, m.number)),
        Some(("r1".to_string(), Some(5)))
    );
}

#[test]
fn match_url_is_the_inverse_of_substitute_over_a_value_grid() {
    for id in IDS {
        for number in [0u64, 1, 5, 42, 123_456] {
            for width in [0usize, 2, 5, MAX_FORMAT_WIDTH] {
                let template = if width == 0 {
                    "$RepresentationID$/$Number$.m4s".to_string()
                } else {
                    format!("$RepresentationID$/$Number%0{width}d$.m4s")
                };
                let url = substitute(&template, id, Some(number), None, 0).unwrap();
                assert_eq!(
                    match_url(&template, &url, IDS),
                    Some(matched(|m| {
                        m.representation_id = (*id).to_string();
                        m.number = Some(number);
                    })),
                    "{template} -> {url}"
                );
            }
            for time in [0u64, 3, 900_000] {
                let template = "$RepresentationID$/t$Time$.m4s";
                let url = substitute(template, id, None, Some(time), 0).unwrap();
                assert_eq!(
                    match_url(template, &url, IDS).map(|m| (m.representation_id, m.time)),
                    Some(((*id).to_string(), Some(time))),
                    "{url}"
                );
            }
            for bandwidth in [0u64, 128_000, 9_999_999] {
                let template = "$RepresentationID$/b$Bandwidth%07d$.m4s";
                let url = substitute(template, id, None, None, bandwidth).unwrap();
                assert_eq!(
                    match_url(template, &url, IDS).map(|m| (m.representation_id, m.bandwidth)),
                    Some(((*id).to_string(), Some(bandwidth))),
                    "{url}"
                );
            }
        }
    }
}

#[test]
fn match_url_backtracks_from_the_longest_digit_run() {
    assert_eq!(
        match_url("$Number$$Number$.m4s", "11.m4s", IDS).map(|m| m.number),
        Some(Some(1))
    );
}

#[test]
fn match_url_returns_none_when_two_readings_fit() {
    assert_eq!(
        match_url("$RepresentationID$$Number$.m4s", "r55.m4s", &["r", "r5"]),
        None
    );
    assert_eq!(
        match_url("$RepresentationID$$Number$.m4s", "r5.m4s", &["r", "r5"])
            .map(|m| (m.representation_id, m.number)),
        Some(("r".to_string(), Some(5)))
    );
}

#[test]
fn match_url_reads_a_dollar_escape() {
    assert_eq!(
        match_url("a$$b-$Number$.m4s", "a$b-7.m4s", IDS),
        Some(matched(|m| m.number = Some(7)))
    );
    assert_eq!(match_url("a$$b-$Number$.m4s", "a$$b-7.m4s", IDS), None);
}

#[test]
fn match_url_refuses_a_width_the_url_does_not_carry() {
    assert_eq!(
        match_url("$RepresentationID$/$Number%03d$.m4s", "r1/05.m4s", IDS),
        None
    );
    assert_eq!(match_url("$Number%05d$.m4s", "1.m4s", IDS), None);
    assert_eq!(
        match_url("$Number%05d$.m4s", "00007.m4s", IDS).map(|m| m.number),
        Some(Some(7))
    );
}

#[test]
fn match_url_refuses_unpadded_leading_zeros() {
    assert_eq!(match_url("$Number$.m4s", "007.m4s", IDS), None);
    assert_eq!(
        match_url("$Number$.m4s", "7.m4s", IDS).map(|m| m.number),
        Some(Some(7))
    );
}

#[test]
fn a_repeated_identifier_must_agree_across_the_url() {
    assert_eq!(match_url("$Number$-$Number$.m4s", "5-6.m4s", IDS), None);
    assert_eq!(
        match_url("$Number$-$Number$.m4s", "5-5.m4s", IDS).map(|m| m.number),
        Some(Some(5))
    );
    assert_eq!(
        match_url(
            "$RepresentationID$-$RepresentationID$.m4s",
            "r1-r1.m4s",
            IDS
        )
        .map(|m| m.representation_id),
        Some("r1".to_string())
    );
    assert_eq!(
        match_url(
            "$RepresentationID$-$RepresentationID$.m4s",
            "r1-r10.m4s",
            IDS
        ),
        None
    );
}

#[test]
fn match_url_refuses_a_template_it_cannot_invert() {
    assert_eq!(match_url("$Number$-$SubNumber$.m4s", "5-6.m4s", IDS), None);
    assert_eq!(match_url("$Number", "5.m4s", IDS), None);
    assert_eq!(match_url("$Foo$", "5.m4s", IDS), None);
    assert_eq!(match_url("$Number$.m4s", "r1/5.m4s", IDS), None);
}

#[test]
fn template_labels_name_every_variant() {
    assert_eq!(
        TemplateError::Invalid(TemplateFault::NumberAndTime).name(),
        "invalid"
    );
    assert_eq!(TemplateError::MissingNumber.name(), "missing_number");
    assert_eq!(TemplateError::MissingTime.name(), "missing_time");
    assert_eq!(
        TemplateError::MissingNumber.fault(),
        TemplateFault::MissingNumberOrTime
    );

    for (fault, label) in [
        (
            TemplateFault::UnterminatedIdentifier,
            "unterminated_identifier",
        ),
        (TemplateFault::UnknownIdentifier, "unknown_identifier"),
        (
            TemplateFault::FormatOnRepresentationId,
            "format_on_representation_id",
        ),
        (TemplateFault::BadFormatTag, "bad_format_tag"),
        (TemplateFault::WidthTooLarge, "width_too_large"),
        (TemplateFault::NumberAndTime, "number_and_time"),
        (TemplateFault::MissingNumberOrTime, "missing_number_or_time"),
        (
            TemplateFault::DynamicIdentifierInInitialization,
            "dynamic_identifier_in_initialization",
        ),
    ] {
        assert_eq!(fault.name(), label);
        assert_eq!(fault.to_string(), label);
    }
    assert_eq!(
        TemplateError::Invalid(TemplateFault::BadFormatTag).to_string(),
        "invalid"
    );
}
