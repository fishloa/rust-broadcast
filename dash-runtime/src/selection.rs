//! Pure selection of AdaptationSets and Representations
//! (`docs/dash-runtime/DESIGN.md` 5.1), by role, language, codec and
//! bandwidth, into the variant model a host speaks.
//!
//! 1. Candidate sets: audio and video ISOBMFF sets that survive the drops of
//!    DESIGN D10 (text, MPEG-2 TS and WebM segments, trick modes, protected
//!    content, an `EssentialProperty` this core does not understand, a
//!    Representation that depends on another, unresolved xlink, invalid
//!    segment information).
//! 2. Video: the first remaining set with Role `main` (else document order);
//!    the Representation is the highest `@bandwidth` the policy allows, else
//!    the lowest. `variants` lists every Representation of every selectable
//!    video set in MPD order.
//! 3. Audio: the first set whose `@lang` matches the preferred language by
//!    BCP 47 prefix and whose Role is not `commentary`/`description`; else
//!    Role `main`; else the first. DASH has no DEFAULT flag, Role `main`
//!    plays that part (5.8.5.5 Table 34).

use broadcast_common::impl_spec_display;

use crate::address::template;
use crate::mpd::descriptors::{self, Role, essential_understood};
use crate::mpd::inherit::{SegmentInfo, effective_common, resolve_segment_info};
use crate::mpd::model::{AdaptationSet, Mpd, Period, Representation};

const CODEC_SEPARATOR: char = ',';
const LANG_SUBTAG_SEPARATOR: char = '-';
const MIME_VIDEO: &str = "video/";
const MIME_AUDIO: &str = "audio/";
const MIME_TEXT: &str = "text/";
const MIME_APPLICATION: &str = "application/";
const MIME_MP2T_SUFFIX: &str = "/mp2t";
const MIME_WEBM_SUFFIX: &str = "/webm";
const CONTENT_VIDEO: &str = "video";
const CONTENT_AUDIO: &str = "audio";
const CONTENT_TEXT: &str = "text";
const CODEC_STPP: &str = "stpp";
const CODEC_WVTT: &str = "wvtt";

/// What a set carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SetClass {
    /// Video.
    Video,
    /// Audio.
    Audio,
    /// Subtitles or captions.
    Text,
    /// Anything else.
    Other,
}

impl SetClass {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Text => "text",
            Self::Other => "other",
        }
    }
}
impl_spec_display!(SetClass);

/// Why a set or Representation was not selectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropReason {
    /// Subtitles/captions (out of scope).
    Text,
    /// Neither audio nor video.
    NotMedia,
    /// MPEG-2 TS segments.
    Mp2tSegments,
    /// WebM segments.
    WebM,
    /// A trick-mode set.
    TrickMode,
    /// Protected, and an unprotected alternative exists in the same group.
    ProtectedAlternativeExists,
    /// Protected with no unprotected alternative.
    UnsupportedEncryption,
    /// An `EssentialProperty` this core does not understand (5.8.4.8).
    UnknownEssentialProperty,
    /// The Representation needs another one to be decoded.
    DependencyRepresentation,
    /// An unresolved xlink.
    UnresolvedXlink,
    /// Segment information violating 5.3.9.1 / 5.3.9.4.4 or out of scope.
    InvalidSegmentInfo,
}

impl DropReason {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::NotMedia => "not_media",
            Self::Mp2tSegments => "mp2t_segments",
            Self::WebM => "webm",
            Self::TrickMode => "trick_mode",
            Self::ProtectedAlternativeExists => "protected_alternative_exists",
            Self::UnsupportedEncryption => "unsupported_encryption",
            Self::UnknownEssentialProperty => "unknown_essential_property",
            Self::DependencyRepresentation => "dependency_representation",
            Self::UnresolvedXlink => "unresolved_xlink",
            Self::InvalidSegmentInfo => "invalid_segment_info",
        }
    }
}
impl_spec_display!(DropReason);

/// A Representation's position in the MPD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RepRef {
    /// Period index.
    pub period: usize,
    /// AdaptationSet index in the Period.
    pub set: usize,
    /// Representation index in the set.
    pub rep: usize,
}

impl RepRef {
    /// The three elements this reference names.
    #[must_use]
    pub fn resolve<'a>(
        &self,
        mpd: &'a Mpd,
    ) -> Option<(&'a Period, &'a AdaptationSet, &'a Representation)> {
        let period = mpd.periods.get(self.period)?;
        let set = period.adaptation_sets.get(self.set)?;
        Some((period, set, set.representations.get(self.rep)?))
    }
}

/// One drop, with its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Dropped {
    /// AdaptationSet index.
    pub set: usize,
    /// Representation index when only one Representation was dropped.
    pub rep: Option<usize>,
    /// Why.
    pub reason: DropReason,
}

/// Selection policy; the same three limits as `hls_runtime::VariantPolicy`
/// plus the audio language.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Policy {
    /// Skip Representations above this bandwidth.
    pub max_bandwidth: Option<u64>,
    /// Skip Representations above this height.
    pub max_height: Option<u32>,
    /// When non-empty, a Representation must carry a codec starting with one
    /// of these prefixes.
    pub codec_prefixes: Vec<String>,
    /// Preferred audio language (BCP 47).
    pub preferred_audio_language: Option<String>,
}

/// An audio AdaptationSet and its Representations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AudioSet {
    /// AdaptationSet index.
    pub set: usize,
    /// Selectable Representations.
    pub reps: Vec<RepRef>,
    /// Role `main` present.
    pub is_main: bool,
}

/// The result of [`select`] for one Period.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Selection {
    /// Every selectable video Representation, MPD order.
    pub variants: Vec<RepRef>,
    /// Index into `variants` of the chosen one.
    pub chosen_variant: Option<usize>,
    /// Selectable audio sets.
    pub audio_sets: Vec<AudioSet>,
    /// The chosen audio Representation.
    pub chosen_audio: Option<RepRef>,
    /// Everything dropped, with reasons.
    pub dropped: Vec<Dropped>,
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Classify a set from `@contentType`, then the MIME types, then codecs.
#[must_use]
pub fn classify(set: &AdaptationSet) -> SetClass {
    let mime = set
        .common
        .mime_type
        .as_deref()
        .or_else(|| {
            set.representations
                .iter()
                .find_map(|r| r.common.mime_type.as_deref())
        })
        .unwrap_or_default();
    let codecs = set
        .common
        .codecs
        .as_deref()
        .or_else(|| {
            set.representations
                .iter()
                .find_map(|r| r.common.codecs.as_deref())
        })
        .unwrap_or_default();
    match set.content_type.as_deref() {
        Some(CONTENT_VIDEO) => return SetClass::Video,
        Some(CONTENT_AUDIO) => return SetClass::Audio,
        Some(CONTENT_TEXT) => return SetClass::Text,
        _ => {}
    }
    if mime.starts_with(MIME_VIDEO) {
        SetClass::Video
    } else if mime.starts_with(MIME_AUDIO) {
        SetClass::Audio
    } else if mime.starts_with(MIME_TEXT)
        || contains_ci(codecs, CODEC_STPP)
        || contains_ci(codecs, CODEC_WVTT)
        || (mime.starts_with(MIME_APPLICATION) && contains_ci(mime, "ttml"))
    {
        SetClass::Text
    } else {
        SetClass::Other
    }
}

fn container_drop(set: &AdaptationSet) -> Option<DropReason> {
    let mimes = std::iter::once(set.common.mime_type.as_deref())
        .chain(set.representations.iter().map(|r| r.common.mime_type.as_deref()))
        .flatten();
    for mime in mimes {
        if mime.ends_with(MIME_MP2T_SUFFIX) {
            return Some(DropReason::Mp2tSegments);
        }
        if mime.ends_with(MIME_WEBM_SUFFIX) {
            return Some(DropReason::WebM);
        }
    }
    None
}

fn has_trick_mode(set: &AdaptationSet) -> bool {
    set.essential_properties
        .iter()
        .chain(&set.supplemental_properties)
        .any(|d| d.scheme_id_uri == transmux::TRICKMODE_SCHEME)
}

fn unknown_essential(descs: &[crate::mpd::model::Descriptor]) -> bool {
    descs.iter().any(|d| !essential_understood(d))
}

fn segment_info_ok(period: &Period, set: &AdaptationSet, rep: &Representation) -> bool {
    match resolve_segment_info(period, set, rep) {
        Ok(SegmentInfo::Template(t)) => {
            let media_ok = t
                .media
                .as_deref()
                .is_some_and(|m| template::scan_media(m).is_ok_and(|u| !u.sub_number));
            let init_ok = t
                .initialization
                .as_deref()
                .is_none_or(|i| template::scan_initialization(i).is_ok_and(|u| !u.sub_number));
            let timeline_ok = t
                .multi
                .timeline
                .as_ref()
                .is_none_or(|tl| tl.unsupported.is_none());
            media_ok && init_ok && timeline_ok
        }
        Ok(SegmentInfo::List(l)) => l
            .multi
            .timeline
            .as_ref()
            .is_none_or(|tl| tl.unsupported.is_none()),
        Ok(SegmentInfo::Base(_)) => true,
        Err(_) => false,
    }
}

/// Whether `preferred` matches `lang` by BCP 47 prefix, ASCII case-insensitive.
#[must_use]
pub fn lang_matches(preferred: &str, lang: &str) -> bool {
    let (p, l) = (preferred.to_ascii_lowercase(), lang.to_ascii_lowercase());
    l == p
        || l.strip_prefix(&p)
            .is_some_and(|rest| rest.starts_with(LANG_SUBTAG_SEPARATOR))
}

fn codec_allowed(codecs: Option<&str>, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return true;
    }
    codecs.is_some_and(|c| {
        c.split(CODEC_SEPARATOR)
            .map(str::trim)
            .any(|tok| prefixes.iter().any(|p| tok.starts_with(p.as_str())))
    })
}

fn is_protected(set: &AdaptationSet) -> bool {
    !set.content_protections.is_empty()
        || (!set.representations.is_empty()
            && set
                .representations
                .iter()
                .all(|r| !r.content_protections.is_empty()))
}

/// Select for `period_idx` of `mpd`.
#[must_use]
pub fn select(mpd: &Mpd, period_idx: usize, policy: &Policy) -> Selection {
    let mut out = Selection::default();
    let Some(period) = mpd.periods.get(period_idx) else {
        return out;
    };
    let classes: Vec<SetClass> = period.adaptation_sets.iter().map(classify).collect();
    let mut survivors: Vec<usize> = Vec::new();
    for (si, set) in period.adaptation_sets.iter().enumerate() {
        let class = classes[si];
        let set_drop = if set.xlink.is_some() {
            Some(DropReason::UnresolvedXlink)
        } else if class == SetClass::Text {
            Some(DropReason::Text)
        } else if class == SetClass::Other {
            Some(DropReason::NotMedia)
        } else if let Some(r) = container_drop(set) {
            Some(r)
        } else if has_trick_mode(set) {
            Some(DropReason::TrickMode)
        } else if unknown_essential(&set.essential_properties) {
            Some(DropReason::UnknownEssentialProperty)
        } else if is_protected(set) {
            let alternative = period.adaptation_sets.iter().enumerate().any(|(oi, other)| {
                oi != si
                    && set.group.is_some()
                    && other.group == set.group
                    && classes[oi] == class
                    && !is_protected(other)
            });
            Some(if alternative {
                DropReason::ProtectedAlternativeExists
            } else {
                DropReason::UnsupportedEncryption
            })
        } else {
            None
        };
        match set_drop {
            Some(reason) => out.dropped.push(Dropped {
                set: si,
                rep: None,
                reason,
            }),
            None => survivors.push(si),
        }
    }

    let mut usable: Vec<(usize, Vec<usize>)> = Vec::new();
    for &si in &survivors {
        let set = &period.adaptation_sets[si];
        let mut reps = Vec::new();
        for (ri, rep) in set.representations.iter().enumerate() {
            let reason = if !rep.dependency_ids.is_empty() {
                Some(DropReason::DependencyRepresentation)
            } else if unknown_essential(&rep.essential_properties) {
                Some(DropReason::UnknownEssentialProperty)
            } else if !rep.content_protections.is_empty() {
                Some(DropReason::UnsupportedEncryption)
            } else if !segment_info_ok(period, set, rep) {
                Some(DropReason::InvalidSegmentInfo)
            } else {
                None
            };
            match reason {
                Some(reason) => out.dropped.push(Dropped {
                    set: si,
                    rep: Some(ri),
                    reason,
                }),
                None => reps.push(ri),
            }
        }
        if !reps.is_empty() {
            usable.push((si, reps));
        }
    }

    let video_sets: Vec<&(usize, Vec<usize>)> = usable
        .iter()
        .filter(|(si, _)| classes[*si] == SetClass::Video)
        .collect();
    for (si, reps) in &video_sets {
        for ri in reps {
            out.variants.push(RepRef {
                period: period_idx,
                set: *si,
                rep: *ri,
            });
        }
    }
    let main_video = video_sets.iter().find(|(si, _)| {
        descriptors::roles(&period.adaptation_sets[*si].roles).any(|r| r == Role::Main)
    });
    if let Some((si, reps)) = main_video.copied().or_else(|| video_sets.first().copied()) {
        let set = &period.adaptation_sets[*si];
        let allowed = |ri: &usize| -> bool {
            let rep = &set.representations[*ri];
            let c = effective_common(set, rep);
            policy.max_bandwidth.is_none_or(|m| rep.bandwidth <= m)
                && policy
                    .max_height
                    .is_none_or(|h| c.height.is_none_or(|rh| rh <= h))
                && codec_allowed(c.codecs.as_deref(), &policy.codec_prefixes)
        };
        let pick = reps
            .iter()
            .filter(|ri| allowed(ri))
            .max_by_key(|ri| set.representations[**ri].bandwidth)
            .or_else(|| {
                reps.iter()
                    .min_by_key(|ri| set.representations[**ri].bandwidth)
            });
        if let Some(ri) = pick {
            out.chosen_variant = out
                .variants
                .iter()
                .position(|v| v.set == *si && v.rep == *ri);
        }
    }

    for (si, reps) in usable
        .iter()
        .filter(|(si, _)| classes[*si] == SetClass::Audio)
    {
        let set = &period.adaptation_sets[*si];
        out.audio_sets.push(AudioSet {
            set: *si,
            reps: reps
                .iter()
                .map(|ri| RepRef {
                    period: period_idx,
                    set: *si,
                    rep: *ri,
                })
                .collect(),
            is_main: descriptors::roles(&set.roles).any(|r| r == Role::Main),
        });
    }
    let secondary = |set: &AdaptationSet| {
        descriptors::roles(&set.roles).any(|r| matches!(r, Role::Commentary | Role::Description))
    };
    let chosen_set = policy
        .preferred_audio_language
        .as_deref()
        .and_then(|pref| {
            out.audio_sets.iter().find(|a| {
                let s = &period.adaptation_sets[a.set];
                s.lang.as_deref().is_some_and(|l| lang_matches(pref, l)) && !secondary(s)
            })
        })
        .or_else(|| out.audio_sets.iter().find(|a| a.is_main))
        .or_else(|| out.audio_sets.first());
    if let Some(a) = chosen_set {
        let set = &period.adaptation_sets[a.set];
        let best = a
            .reps
            .iter()
            .filter(|r| {
                let rep = &set.representations[r.rep];
                codec_allowed(
                    effective_common(set, rep).codecs.as_deref(),
                    &policy.codec_prefixes,
                )
            })
            .max_by_key(|r| set.representations[r.rep].bandwidth)
            .or_else(|| a.reps.first());
        out.chosen_audio = best.copied();
    }
    out
}
