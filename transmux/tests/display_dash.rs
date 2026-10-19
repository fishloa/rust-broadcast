//! DASH display geometry (Phase 4c): @sar on Representation, @par on AdaptationSet.

#![cfg(feature = "std")]

use broadcast_common::{Package, Unpackage};
use transmux::DashPackager;
use transmux::ts_demux::TsDemux;

fn mpd_for(fixture: &str) -> Option<String> {
    let data = test_fixtures::require_bytes(fixture, "display_dash")?;
    let media = TsDemux::new().unpackage(&data).ok()?;
    DashPackager::default().package(&media).ok()
}

fn contains_attr(mpd: &str, attr: &str, val: &str) -> bool {
    mpd.contains(&format!("{attr}=\"{val}\""))
}

#[test]
fn melodie_sar_and_par() {
    let Some(mpd) = mpd_for("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    assert!(contains_attr(&mpd, "sar", "64:45"));
    assert!(contains_attr(&mpd, "par", "16:9"));
}

#[test]
fn krone_sar_and_par() {
    let Some(mpd) = mpd_for("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    assert!(contains_attr(&mpd, "sar", "16:11"));
    assert!(contains_attr(&mpd, "par", "20:11"));
}

#[test]
fn square_pixel_sar() {
    let Some(mpd) = mpd_for("ts/france-tnt-m6-eac3-zenith.ts") else {
        return;
    };
    assert!(contains_attr(&mpd, "sar", "1:1"));
}
