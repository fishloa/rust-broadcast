//! End-to-end SSAI session walkthrough: decide an ad break from a real
//! SCTE-35 cue, condition its splice point, track it in a
//! [`ssai_runtime::session::SessionStore`], and render one viewer's
//! Interstitial playlist over `broadcast-hls` — without touching the
//! playlist every other viewer sees.
//!
//! ```sh
//! cargo run -p ssai-runtime --example session_playlist
//! ```
use broadcast_common::Parse;
use broadcast_hls::{MediaPlaylist, MediaSegment};
use mp4_emsg::{EmsgBox, PresentationTime};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::AnyCommand;
use ssai_runtime::decision::{AdBreakDecision, AdDecisionProvider, BreakContext, RestrictMode};
use ssai_runtime::playlist::{InterstitialDateRange, render_session_playlist};
use ssai_runtime::session::{BreakState, SessionStore};
use ssai_runtime::splice::condition_splice_point;
use std::fs;

/// A toy decision provider: always the same single-asset interstitial. A
/// real implementation would call out to a VAST/VMAP ad server here — this
/// crate's `AdDecisionProvider` trait does no I/O itself, so that call is
/// entirely this impl's business, not `ssai-runtime`'s (issue #929 design
/// decision: no HTTP client in this crate).
struct FixedProvider;

impl AdDecisionProvider for FixedProvider {
    fn decide(&self, ctx: &BreakContext) -> ssai_runtime::Result<AdBreakDecision> {
        let mut decision = AdBreakDecision::single_asset(
            format!("break-{}", ctx.splice_event_id.unwrap_or(0)),
            "https://ads.example.com/creative-123.m3u8",
        );
        decision.resume_offset = Some(0.0);
        decision.restrict = vec![RestrictMode::Skip, RestrictMode::Jump];
        Ok(decision)
    }
}

fn main() {
    // Decode the same real cue `condition_real_cue` uses, so this walkthrough
    // is grounded in genuine SCTE-35 fields rather than invented ones.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/scte35-ssai/emsg_splice_insert.bin"
    );
    let emsg_bytes = fs::read(path).expect("read real emsg fixture (see PROVENANCE.md)");
    let emsg = EmsgBox::parse(&emsg_bytes).expect("parse real emsg box");
    let requested_pts = match emsg.presentation_time {
        PresentationTime::Absolute(t) => t,
        other => panic!("fixture is expected to be a v1 (Absolute) emsg, got {other:?}"),
    };
    let section = SpliceInfoSection::parse(emsg.message_data).expect("parse splice_info_section");
    let clear = section.clear.as_ref().expect("cue is not encrypted");
    let AnyCommand::SpliceInsert(si) = &clear.command else {
        panic!("unexpected command in this fixture")
    };

    let ctx = BreakContext {
        session_id: "viewer-42".to_string(),
        splice_event_id: Some(si.splice_event_id),
        out_of_network: si.out_of_network_indicator,
        break_duration_ticks: si.break_duration.as_ref().map(|bd| bd.duration),
        splice_point_pts: requested_pts,
    };
    println!("break context: {ctx:?}");

    let decision = FixedProvider.decide(&ctx).expect("decision");
    println!("decision: {decision:?}");

    // Condition against the same independently-measured real keyframe
    // `condition_real_cue` verifies (PROVENANCE.md's "Cue-to-IDR alignment").
    let conditioned = condition_splice_point(requested_pts, &[160_767_315_906_000u64], 10_000)
        .expect("real cue is within a 111ms tolerance");
    println!("conditioned splice point: {conditioned:?}");

    let mut sessions = SessionStore::new();
    sessions
        .begin_break(
            ctx.session_id.clone(),
            BreakState::new(decision.clone(), conditioned),
        )
        .expect("no break already active for this session");

    let duration_s = ctx.break_duration_ticks.map(|t| t as f64 / 90_000.0);
    let dr =
        InterstitialDateRange::from_decision(&decision, "2026-08-09T19:25:10.000Z", duration_s);

    let mut base = MediaPlaylist {
        target_duration: 6,
        ..Default::default()
    };
    base.segments.push(MediaSegment {
        duration: 6.0,
        uri: "main0.ts".to_string(),
        ..Default::default()
    });

    let rendered_for_this_viewer = render_session_playlist(&base, Some(&dr));
    let rendered_for_everyone_else = render_session_playlist(&base, None);

    println!("\n--- viewer-42's rendered playlist (in the break) ---");
    println!("{}", rendered_for_this_viewer.to_m3u8());
    println!("--- every other viewer's rendered playlist (no break) ---");
    println!("{}", rendered_for_everyone_else.to_m3u8());

    assert!(
        rendered_for_this_viewer
            .to_m3u8()
            .contains("X-ASSET-URI=\"https://ads.example.com/creative-123.m3u8\"")
    );
    assert!(!rendered_for_everyone_else.to_m3u8().contains("X-ASSET-URI"));
    assert_eq!(sessions.len(), 1);
    println!("OK: session tracked, interstitial rendered into this viewer's playlist only.");
}
