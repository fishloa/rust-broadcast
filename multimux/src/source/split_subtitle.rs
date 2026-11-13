//! WebVTT subtitle renditions from DVB teletext (issue #1428).
//!
//! A teletext PID arrives as a `CodecConfig::Data` track whose PMT ES-info
//! loop carries the teletext descriptor (ETSI EN 300 468 §6.2.43, tag 0x56).
//! Every page it lists as a subtitle page (`teletext_type` 0x02) or a subtitle
//! page for hearing impaired people (0x05) becomes one `EXT-X-MEDIA
//! TYPE=SUBTITLES` rendition, declared as soon as the track set shows the
//! track, and served from its own sibling `Trunk`/`HlsOrigin` with
//! `Container::WebVtt`.
//!
//! # Timing
//!
//! Subtitle segment N covers exactly the span of leader segment N and carries
//! its media sequence number, including when it has no cue (RFC 8216 §3.5).
//! The follower learns the span from the leader's part cuts and segment
//! durations, never from the wall clock.
//!
//! All segments share one `X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0`: the
//! leader's fMP4 timeline starts at zero at the first decode time it cut on
//! (the follower's timeline origin, the same one the split-audio followers
//! use), so cue time `t` is the cue's PTS minus that origin and
//! `LOCAL 0` is output time `0` ([`MPEGTS_AT_ORIGIN`]).
//!
//! # Partial segments and open cues
//!
//! On a low-latency route every subtitle rendition publishes Partial Segments
//! cut exactly where the leader cut its own, and the parent segment when the
//! leader closes its segment. Nothing is held back: a part or segment is
//! published at the leader's cut with the cues the decoder has by then.
//!
//! Each part is a complete WebVTT document with the same header and
//! `X-TIMESTAMP-MAP` as the segments, holding the cues displayed during its
//! time range (RFC 8216bis §3.1.4). A cue that has closed is written with its
//! own start and end whatever part of it falls outside the range. A cue still
//! on screen has no end yet, so it is written with its end clamped to the end
//! of the range, and again, with its original start, in every following part
//! and segment until it closes, when its real end is used.

use hls_runtime::server::SubtitleRenditionDecl;
use media_plane::segmenter::subtitle::{SubtitleFollower, TeletextPage, teletext_pages};
use media_plane::trunk::Trunk;
use transmux::pipeline::TrackSpec;

use crate::route::{MAX_SUBTITLE_RENDITIONS, RouteHandle, SubtitleServing, subtitle_playlist_uri};

/// Suffix of the `NAME` of a hearing-impaired page.
const HEARING_IMPAIRED_SUFFIX: &str = " (SDH)";

/// The rendition declarations for `pages`: `NAME` is the language (with
/// ` (SDH)` for a hearing-impaired page), plus the page number where two
/// pages would otherwise share it.
pub(crate) fn declarations(pages: &[TeletextPage]) -> Vec<SubtitleRenditionDecl> {
    let base: Vec<String> = pages
        .iter()
        .map(|p| {
            let suffix = if p.hearing_impaired {
                HEARING_IMPAIRED_SUFFIX
            } else {
                ""
            };
            format!("{}{suffix}", p.language)
        })
        .collect();
    pages
        .iter()
        .enumerate()
        .map(|(slot, page)| {
            let clash = base.iter().filter(|b| **b == base[slot]).count() > 1;
            let name = if clash {
                format!("{} {}", base[slot], page.number())
            } else {
                base[slot].clone()
            };
            let mut decl = SubtitleRenditionDecl::new(
                name,
                page.language.clone(),
                subtitle_playlist_uri(slot),
            );
            decl.hearing_impaired = page.hearing_impaired;
            let repeats_earlier = base[..slot].contains(&base[slot]);
            decl.autoselect = !repeats_earlier;
            decl
        })
        .collect()
}

/// Declares the subtitle renditions of `trunk`'s program when the route wants
/// them and the track set shows teletext subtitle pages, and returns the
/// pages with their servings.
pub(crate) fn resolve(
    route: &RouteHandle,
    trunk: &Trunk,
    tracks: &[TrackSpec],
) -> Vec<(TeletextPage, SubtitleServing)> {
    if !route.subtitles_enabled() || route.container() != transmux::Container::Fmp4 {
        return Vec::new();
    }
    let pages = teletext_pages(route.name(), MAX_SUBTITLE_RENDITIONS, tracks);
    if pages.is_empty() {
        return Vec::new();
    }
    let decls = declarations(&pages);
    let identities: Vec<String> = pages.iter().map(TeletextPage::identity).collect();
    let Some(servings) = route.declare_subtitles_for_trunk(trunk, &decls, &identities) else {
        return Vec::new();
    };
    let mut taken = vec![false; servings.len()];
    let mut paired = Vec::new();
    for page in pages {
        let identity = page.identity();
        let slot = servings
            .iter()
            .enumerate()
            .find(|(i, s)| !taken[*i] && s.identity == identity)
            .map(|(i, _)| i);
        match slot {
            Some(i) => {
                taken[i] = true;
                paired.push((page, servings[i].clone()));
            }
            None => tracing::warn!(
                route = route.name(),
                page = %page.number(),
                language = %page.language,
                "teletext subtitle page has no declared rendition; not served"
            ),
        }
    }
    paired
}

/// Builds one follower per declared page.
pub(super) fn build_followers(
    route: &RouteHandle,
    trunk: &Trunk,
    tracks: &[TrackSpec],
) -> Vec<SubtitleFollower> {
    resolve(route, trunk, tracks)
        .into_iter()
        .filter_map(|(page, serving)| {
            let spec = tracks.iter().find(|t| t.track_id == page.source_track)?;
            SubtitleFollower::try_new(&page, spec.timescale, &serving.trunk)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_si::descriptors::teletext::TeletextEntry;
    use dvb_si::descriptors::{AnyDescriptor, parse_loop};
    use transmux::TrackId;
    use transmux::pipeline::CodecConfig;

    #[test]
    fn page_names_are_unique_and_flag_hearing_impaired() {
        let page = |lang: &str, sdh: bool, page: u8| TeletextPage {
            source_track: TrackId::new(1),
            language: lang.into(),
            hearing_impaired: sdh,
            magazine: 8,
            page,
        };
        let decls = declarations(&[page("fr", true, 0x88), page("fr", false, 0x89)]);
        assert_eq!(decls[0].name, "fr (SDH)");
        assert!(decls[0].hearing_impaired);
        assert_eq!(decls[1].name, "fr");
        assert!(!decls[1].hearing_impaired);
        assert_eq!(decls[1].uri, "subs-1.m3u8");

        let clash = declarations(&[page("fr", false, 0x89), page("fr", false, 0x90)]);
        assert_eq!(clash[0].name, "fr 889");
        assert_eq!(clash[1].name, "fr 890");
    }

    const TELETEXT_FIXTURE: &str = "input/test-012.ts";

    fn teletext_track() -> Option<TrackSpec> {
        use broadcast_common::mux::Unpackage;
        let path = test_fixtures::tsduck(TELETEXT_FIXTURE, "split_subtitle track change")?;
        let bytes = std::fs::read(path).expect("read teletext fixture");
        let media = transmux::ts_demux::TsDemux::new()
            .unpackage(&bytes)
            .expect("demux fixture");
        media
            .tracks
            .into_iter()
            .map(|t| t.spec)
            .find(|spec| matches!(spec.config, CodecConfig::Data { .. }))
    }

    fn with_entries(base: &TrackSpec, entries: Vec<TeletextEntry>) -> TrackSpec {
        use broadcast_common::Serialize;
        let descriptor = dvb_si::descriptors::teletext::TeletextDescriptor { entries };
        let mut bytes = vec![0u8; descriptor.serialized_len()];
        descriptor.serialize_into(&mut bytes).expect("serialize");
        let mut spec = base.clone();
        spec.es_info_descriptors = bytes;
        spec
    }

    fn fixture_entries(spec: &TrackSpec) -> Vec<TeletextEntry> {
        parse_loop(&spec.es_info_descriptors)
            .flatten()
            .find_map(|d| match d {
                AnyDescriptor::Teletext(t) => Some(t.entries),
                _ => None,
            })
            .expect("fixture teletext descriptor")
    }

    fn rig() -> (RouteHandle, std::sync::Arc<Trunk>) {
        use media_plane::trunk::TrunkConfig;
        use std::num::NonZeroUsize;
        let nz = |n| NonZeroUsize::new(n).expect("non-zero");
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(8))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(64)),
        );
        let route = RouteHandle::new(2.0, 500, 4).with_subtitles(true);
        route.publish_program(media_plane::ProgramId(0), std::sync::Arc::clone(&trunk));
        (route, trunk)
    }

    fn served(pairs: &[(TeletextPage, SubtitleServing)]) -> Vec<(String, bool, String)> {
        pairs
            .iter()
            .map(|(page, serving)| {
                (
                    page.number(),
                    serving.decl.hearing_impaired,
                    serving.decl.uri.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn a_track_change_matches_pages_to_renditions_by_identity() {
        let Some(base) = teletext_track() else {
            return;
        };
        let entries = fixture_entries(&base);
        assert_eq!(entries.len(), 2, "fixture lists an SDH and a plain page");
        let (route, trunk) = rig();

        let first = resolve(&route, &trunk, &[with_entries(&base, entries.clone())]);
        let initial = served(&first);
        assert_eq!(
            initial,
            [
                ("888".to_string(), true, "subs-0.m3u8".to_string()),
                ("889".to_string(), false, "subs-1.m3u8".to_string()),
            ]
        );

        let reordered = vec![entries[1], entries[0]];
        let after_reorder = resolve(&route, &trunk, &[with_entries(&base, reordered)]);
        let mut by_page = served(&after_reorder);
        by_page.sort();
        assert_eq!(by_page, initial);

        let dropped = vec![entries[1]];
        let after_drop = resolve(&route, &trunk, &[with_entries(&base, dropped)]);
        assert_eq!(
            served(&after_drop),
            [("889".to_string(), false, "subs-1.m3u8".to_string())]
        );

        let mut extra = entries[1];
        extra.page_number = 0x90;
        let after_add = resolve(
            &route,
            &trunk,
            &[with_entries(&base, vec![entries[0], extra, entries[1]])],
        );
        assert_eq!(
            served(&after_add),
            [
                ("888".to_string(), true, "subs-0.m3u8".to_string()),
                ("889".to_string(), false, "subs-1.m3u8".to_string()),
            ]
        );
    }

    #[test]
    fn a_page_that_changes_language_is_not_served_under_the_old_rendition() {
        let Some(base) = teletext_track() else {
            return;
        };
        let entries = fixture_entries(&base);
        let (route, trunk) = rig();
        resolve(&route, &trunk, &[with_entries(&base, entries.clone())]);
        let mut german = entries[1];
        german.language_code = dvb_si::text::LangCode(*b"deu");
        let after = resolve(&route, &trunk, &[with_entries(&base, vec![german])]);
        assert!(served(&after).is_empty());
    }

    #[test]
    fn a_later_page_repeating_a_language_is_not_autoselect() {
        let page = |sdh: bool, page: u8| TeletextPage {
            source_track: TrackId::new(1),
            language: "fr".into(),
            hearing_impaired: sdh,
            magazine: 8,
            page,
        };
        let decls = declarations(&[page(false, 0x89), page(false, 0x90), page(true, 0x91)]);
        assert!(decls[0].autoselect);
        assert!(!decls[1].autoselect);
        assert!(decls[2].autoselect);
    }
}
