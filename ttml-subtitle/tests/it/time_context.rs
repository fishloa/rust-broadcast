//! `ttml_subtitle::parse` and the `ttp:*` parameter-attribute to
//! [`TimeContext`] derivation (#3076): every non-default arm of
//! `TtElement::time_context` is asserted through the public entry point.
#![cfg(feature = "std")]

use ttml_subtitle::Document;
use ttml_subtitle::document::{NS_TT, NS_TTP};
use ttml_subtitle::time::{ClockMode, DropMode, MarkerMode, TimeBase, TimeContext};

const FRAME_PARAMS: &str = r#"ttp:frameRate="30" ttp:frameRateMultiplier="1000 1001""#;

fn parse_with(params: &str) -> Document {
    let xml = format!(r#"<tt xmlns="{NS_TT}" xmlns:ttp="{NS_TTP}" {FRAME_PARAMS} {params}/>"#);
    ttml_subtitle::parse(&xml).unwrap_or_else(|e| panic!("parse failed for '{params}': {e}"))
}

fn expected() -> TimeContext {
    TimeContext {
        time_base: TimeBase::Media,
        frame_rate: 30,
        frame_rate_multiplier_numerator: 1000,
        frame_rate_multiplier_denominator: 1001,
        sub_frame_rate: 1,
        tick_rate: 29,
        drop_mode: DropMode::NonDrop,
        marker_mode: MarkerMode::Discontinuous,
        clock_mode: ClockMode::Utc,
    }
}

#[test]
fn frame_rate_and_multiplier_drive_tick_rate() {
    let ctx = parse_with("").time_context();
    assert_eq!(ctx, expected());
}

#[test]
fn time_base_smpte_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="smpte""#).time_context();
    assert_eq!(ctx.time_base, TimeBase::Smpte);
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Smpte,
            ..expected()
        }
    );
}

#[test]
fn time_base_clock_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="clock""#).time_context();
    assert_eq!(ctx.time_base, TimeBase::Clock);
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Clock,
            ..expected()
        }
    );
}

#[test]
fn drop_mode_ntsc_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="smpte" ttp:dropMode="dropNTSC""#).time_context();
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Smpte,
            drop_mode: DropMode::DropNtsc,
            ..expected()
        }
    );
}

#[test]
fn drop_mode_pal_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="smpte" ttp:dropMode="dropPAL""#).time_context();
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Smpte,
            drop_mode: DropMode::DropPal,
            ..expected()
        }
    );
}

#[test]
fn marker_mode_continuous_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="smpte" ttp:markerMode="continuous""#).time_context();
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Smpte,
            marker_mode: MarkerMode::Continuous,
            ..expected()
        }
    );
}

#[test]
fn clock_mode_local_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="clock" ttp:clockMode="local""#).time_context();
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Clock,
            clock_mode: ClockMode::Local,
            ..expected()
        }
    );
}

#[test]
fn clock_mode_gps_is_derived() {
    let ctx = parse_with(r#"ttp:timeBase="clock" ttp:clockMode="gps""#).time_context();
    assert_eq!(
        ctx,
        TimeContext {
            time_base: TimeBase::Clock,
            clock_mode: ClockMode::Gps,
            ..expected()
        }
    );
}

#[test]
fn absent_parameters_use_ttml_defaults() {
    let xml = format!(r#"<tt xmlns="{NS_TT}"/>"#);
    let doc = ttml_subtitle::parse(&xml).expect("parse");
    assert_eq!(
        doc.time_context(),
        TimeContext {
            time_base: TimeBase::Media,
            frame_rate: 30,
            frame_rate_multiplier_numerator: 1,
            frame_rate_multiplier_denominator: 1,
            sub_frame_rate: 1,
            tick_rate: 1,
            drop_mode: DropMode::NonDrop,
            marker_mode: MarkerMode::Discontinuous,
            clock_mode: ClockMode::Utc,
        }
    );
}

#[test]
fn free_parse_matches_document_parse_str() {
    let xml = format!(r#"<tt xmlns="{NS_TT}" xml:lang="en"><body/></tt>"#);
    let free = ttml_subtitle::parse(&xml).expect("free parse");
    let method = Document::parse_str(&xml).expect("method parse");
    assert_eq!(free, method);
    assert_eq!(free.tt.xml_lang.as_deref(), Some("en"));

    assert_eq!(ttml_subtitle::parse("<tt/>"), Document::parse_str("<tt/>"));
}

#[test]
fn free_parse_rejects_malformed_documents() {
    assert!(matches!(
        ttml_subtitle::parse("<tt"),
        Err(ttml_subtitle::Error::XmlParse(_))
    ));
    assert!(matches!(
        ttml_subtitle::parse("<not-tt/>"),
        Err(ttml_subtitle::Error::NotTtmlRoot(_))
    ));
}
