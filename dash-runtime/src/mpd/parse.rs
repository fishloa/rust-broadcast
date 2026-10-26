//! MPD element tree to typed model (ISO/IEC 23009-1 clause 5.3, Annex B).

use std::time::Duration;

use transmux::parse_iso8601_duration;

use super::descriptors::XLINK_RESOLVE_TO_ZERO;
use super::model::*;
use super::xml::{Elem, Ns, read_tree};
use crate::error::{Limit, MpdError, Unsupported};
use crate::limits::Limits;
use crate::timing::datetime::parse_xs_datetime;
use crate::timing::utc_timing::UtcScheme;

const PROFILE_SEPARATOR: char = ',';
const INFINITY_TOKEN: &str = "INF";
const NEGATIVE_INFINITY_TOKEN: &str = "-INF";
const MAX_START_WITH_SAP: u8 = 6;

fn invalid(what: &'static str, detail: impl Into<String>) -> MpdError {
    MpdError::Invalid {
        what,
        detail: detail.into(),
    }
}

fn opt_str(e: &Elem, name: &str) -> Option<String> {
    e.attr(name).map(str::to_owned)
}

fn opt_parse<T: core::str::FromStr>(e: &Elem, name: &'static str) -> Result<Option<T>, MpdError> {
    match e.attr(name) {
        None => Ok(None),
        Some(v) => v
            .trim()
            .parse::<T>()
            .map(Some)
            .map_err(|_| invalid(name, v)),
    }
}

fn opt_u64(e: &Elem, name: &'static str) -> Result<Option<u64>, MpdError> {
    opt_parse(e, name)
}

fn opt_u32(e: &Elem, name: &'static str) -> Result<Option<u32>, MpdError> {
    opt_parse(e, name)
}

fn opt_i64(e: &Elem, name: &'static str) -> Result<Option<i64>, MpdError> {
    opt_parse(e, name)
}

fn positive_u64(e: &Elem, name: &'static str) -> Result<Option<u64>, MpdError> {
    match opt_u64(e, name)? {
        Some(0) => Err(invalid(name, "must be greater than zero")),
        other => Ok(other),
    }
}

fn parse_bool(name: &'static str, v: &str) -> Result<bool, MpdError> {
    match v.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(invalid(name, other)),
    }
}

fn opt_bool(e: &Elem, name: &'static str) -> Result<Option<bool>, MpdError> {
    e.attr(name).map(|v| parse_bool(name, v)).transpose()
}

fn opt_f64(e: &Elem, name: &'static str) -> Result<Option<f64>, MpdError> {
    let Some(v) = e.attr(name) else {
        return Ok(None);
    };
    let v = v.trim();
    if v == INFINITY_TOKEN {
        return Ok(Some(f64::INFINITY));
    }
    if v == NEGATIVE_INFINITY_TOKEN {
        return Ok(Some(f64::NEG_INFINITY));
    }
    match v.parse::<f64>() {
        Ok(x) if x.is_finite() => Ok(Some(x)),
        _ => Err(invalid(name, v)),
    }
}

fn opt_duration(e: &Elem, name: &'static str) -> Result<Option<Duration>, MpdError> {
    e.attr(name)
        .map(|v| parse_iso8601_duration(v).map_err(|_| invalid(name, v)))
        .transpose()
}

fn opt_time(e: &Elem, name: &'static str) -> Result<Option<i64>, MpdError> {
    e.attr(name)
        .map(|v| {
            parse_xs_datetime(v)
                .map(|t| t.unix_us)
                .map_err(|_| invalid(name, v))
        })
        .transpose()
}

fn parse_range(name: &'static str, v: &str) -> Result<ByteRange, MpdError> {
    let v = v.trim();
    let (first, last) = v.split_once('-').ok_or_else(|| invalid(name, v))?;
    let first = first.parse::<u64>().map_err(|_| invalid(name, v))?;
    let last = if last.is_empty() {
        None
    } else {
        Some(last.parse::<u64>().map_err(|_| invalid(name, v))?)
    };
    if let Some(l) = last
        && l < first
    {
        return Err(invalid(name, v));
    }
    Ok(ByteRange { first, last })
}

fn opt_range(e: &Elem, name: &'static str) -> Result<Option<ByteRange>, MpdError> {
    e.attr(name).map(|v| parse_range(name, v)).transpose()
}

fn xlink(e: &Elem) -> Result<Option<Xlink>, MpdError> {
    let Some(href) = e.attr_in(Ns::Xlink, "href") else {
        return Ok(None);
    };
    let actuate = match e.attr_in(Ns::Xlink, "actuate") {
        None | Some("onRequest") => Actuate::OnRequest,
        Some("onLoad") => Actuate::OnLoad,
        Some(other) => return Err(invalid("xlink:actuate", other)),
    };
    Ok(Some(Xlink {
        href: href.to_owned(),
        actuate,
    }))
}

fn is_resolve_to_zero(x: &Option<Xlink>) -> bool {
    x.as_ref().is_some_and(|x| x.href == XLINK_RESOLVE_TO_ZERO)
}

struct Ctx<'a> {
    limits: &'a Limits,
    features: MpdFeatures,
}

fn check_count(count: usize, max: usize, limit: Limit) -> Result<(), MpdError> {
    if count > max {
        Err(MpdError::TooLarge { limit, max })
    } else {
        Ok(())
    }
}

fn descriptor(e: &Elem) -> Result<Descriptor, MpdError> {
    let scheme_id_uri = e
        .attr("schemeIdUri")
        .ok_or_else(|| invalid("schemeIdUri", format!("missing on {}", e.name)))?
        .to_owned();
    let url_query = e
        .children
        .iter()
        .find(|c| c.ns == Ns::UrlParam && c.name == "UrlQueryInfo")
        .map(|q| -> Result<UrlQueryInfo, MpdError> {
            Ok(UrlQueryInfo {
                use_mpd_url_query: opt_bool(q, "useMPDUrlQuery")?.unwrap_or(false),
                query_string: opt_str(q, "queryString"),
                query_template: opt_str(q, "queryTemplate"),
            })
        })
        .transpose()?;
    Ok(Descriptor {
        scheme_id_uri,
        value: opt_str(e, "value"),
        id: opt_str(e, "id"),
        default_kid: e.attr_in(Ns::Cenc, "default_KID").map(str::to_owned),
        url_query,
    })
}

fn descriptors(e: &Elem, name: &str) -> Result<Vec<Descriptor>, MpdError> {
    e.children_named(name).map(descriptor).collect()
}

fn base_urls(e: &Elem, ctx: &mut Ctx<'_>) -> Result<Vec<BaseUrl>, MpdError> {
    let mut out = Vec::new();
    for b in e.children_named("BaseURL") {
        check_count(out.len() + 1, ctx.limits.max_base_urls, Limit::BaseUrls)?;
        let ato = opt_f64(b, "availabilityTimeOffset")?;
        let complete = opt_bool(b, "availabilityTimeComplete")?;
        if ato.is_some() || complete == Some(false) {
            ctx.features.low_latency = true;
        }
        out.push(BaseUrl {
            url: b.text.trim().to_owned(),
            service_location: opt_str(b, "serviceLocation"),
            priority: dvb_u32(b, "priority")?,
            weight: dvb_u32(b, "weight")?,
            availability_time_offset: ato,
            availability_time_complete: complete,
            time_shift_buffer_depth: opt_duration(b, "timeShiftBufferDepth")?,
        });
    }
    Ok(out)
}

fn dvb_u32(e: &Elem, name: &'static str) -> Result<Option<u32>, MpdError> {
    e.attr_in(Ns::Dvb, name)
        .map(|v| v.trim().parse::<u32>().map_err(|_| invalid(name, v)))
        .transpose()
}

fn url_type(e: &Elem) -> Result<UrlType, MpdError> {
    Ok(UrlType {
        source_url: opt_str(e, "sourceURL"),
        range: opt_range(e, "range")?,
    })
}

fn base_info(e: &Elem, ctx: &mut Ctx<'_>) -> Result<BaseInfo, MpdError> {
    let ato = opt_f64(e, "availabilityTimeOffset")?;
    let complete = opt_bool(e, "availabilityTimeComplete")?;
    if ato.is_some() || complete == Some(false) {
        ctx.features.low_latency = true;
    }
    Ok(BaseInfo {
        timescale: positive_u64(e, "timescale")?,
        presentation_time_offset: opt_u64(e, "presentationTimeOffset")?,
        ept_delta: opt_i64(e, "eptDelta")?,
        pd_delta: opt_i64(e, "pdDelta")?,
        presentation_duration: opt_u64(e, "presentationDuration")?,
        time_shift_buffer_depth: opt_duration(e, "timeShiftBufferDepth")?,
        index_range: opt_range(e, "indexRange")?,
        index_range_exact: opt_bool(e, "indexRangeExact")?,
        availability_time_offset: ato,
        availability_time_complete: complete,
        initialization: e.child("Initialization").map(url_type).transpose()?,
        representation_index: e.child("RepresentationIndex").map(url_type).transpose()?,
    })
}

fn timeline(e: &Elem, ctx: &Ctx<'_>) -> Result<SegmentTimeline, MpdError> {
    let mut out = SegmentTimeline::default();
    if e.has_child("Pattern") {
        out.unsupported = Some(Unsupported::SegmentSequence);
    }
    for s in e.children_named("S") {
        check_count(
            out.entries.len() + 1,
            ctx.limits.max_timeline_entries,
            Limit::TimelineEntries,
        )?;
        let d = opt_u64(s, "d")?.ok_or_else(|| invalid("S@d", "missing"))?;
        if d == 0 {
            return Err(invalid("S@d", "must be greater than zero"));
        }
        let k = opt_u64(s, "k")?;
        if k.is_some_and(|k| k > 1) {
            out.unsupported = Some(Unsupported::SegmentSequence);
        }
        out.entries.push(TimelineEntry {
            t: opt_u64(s, "t")?,
            n: opt_u64(s, "n")?,
            d,
            r: opt_i64(s, "r")?.unwrap_or(0),
            k,
        });
    }
    Ok(out)
}

fn multi_info(e: &Elem, ctx: &mut Ctx<'_>) -> Result<MultiInfo, MpdError> {
    Ok(MultiInfo {
        base: base_info(e, ctx)?,
        duration: positive_u64(e, "duration")?,
        start_number: opt_u64(e, "startNumber")?,
        end_number: opt_u64(e, "endNumber")?,
        timeline: e
            .child("SegmentTimeline")
            .map(|t| timeline(t, ctx))
            .transpose()?,
        bitstream_switching: e.child("BitstreamSwitching").map(url_type).transpose()?,
    })
}

fn segment_layer(e: &Elem, ctx: &mut Ctx<'_>) -> Result<SegmentLayer, MpdError> {
    let mut layer = SegmentLayer::default();
    let count = ["SegmentBase", "SegmentList", "SegmentTemplate"]
        .iter()
        .map(|n| e.children_named(n).count())
        .sum::<usize>();
    if count > 1 {
        return Err(invalid(
            "Segment information",
            format!(
                "more than one of SegmentBase/SegmentList/SegmentTemplate in {}",
                e.name
            ),
        ));
    }
    if let Some(b) = e.child("SegmentBase") {
        layer.base = Some(SegmentBase {
            info: base_info(b, ctx)?,
        });
    }
    if let Some(l) = e.child("SegmentList") {
        let mut urls = Vec::new();
        for u in l.children_named("SegmentURL") {
            check_count(
                urls.len() + 1,
                ctx.limits.max_segment_urls,
                Limit::SegmentUrls,
            )?;
            urls.push(SegmentUrl {
                media: opt_str(u, "media"),
                media_range: opt_range(u, "mediaRange")?,
                index: opt_str(u, "index"),
                index_range: opt_range(u, "indexRange")?,
            });
        }
        layer.list = Some(SegmentList {
            multi: multi_info(l, ctx)?,
            xlink: xlink(l)?,
            urls,
        });
    }
    if let Some(t) = e.child("SegmentTemplate") {
        layer.template = Some(SegmentTemplate {
            multi: multi_info(t, ctx)?,
            media: opt_str(t, "media"),
            index: opt_str(t, "index"),
            initialization: opt_str(t, "initialization"),
            bitstream_switching: opt_str(t, "bitstreamSwitching"),
        });
    }
    Ok(layer)
}

fn common(e: &Elem) -> Result<CommonAttributes, MpdError> {
    Ok(CommonAttributes {
        profiles: opt_str(e, "profiles"),
        width: opt_u32(e, "width")?,
        height: opt_u32(e, "height")?,
        sar: opt_str(e, "sar"),
        frame_rate: opt_str(e, "frameRate"),
        audio_sampling_rate: opt_str(e, "audioSamplingRate"),
        mime_type: opt_str(e, "mimeType"),
        codecs: opt_str(e, "codecs"),
        max_playout_rate: opt_f64(e, "maxPlayoutRate")?,
    })
}

fn representation(e: &Elem, ctx: &mut Ctx<'_>) -> Result<Representation, MpdError> {
    let id = e
        .attr("id")
        .ok_or_else(|| invalid("Representation@id", "missing"))?
        .to_owned();
    let bandwidth = opt_u64(e, "bandwidth")?
        .ok_or_else(|| invalid("Representation@bandwidth", format!("missing on {id}")))?;
    Ok(Representation {
        id,
        bandwidth,
        quality_ranking: opt_u32(e, "qualityRanking")?,
        dependency_ids: e
            .attr("dependencyId")
            .map(|v| v.split_ascii_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        common: common(e)?,
        base_urls: base_urls(e, ctx)?,
        segment: segment_layer(e, ctx)?,
        content_protections: descriptors(e, "ContentProtection")?,
        essential_properties: descriptors(e, "EssentialProperty")?,
        supplemental_properties: descriptors(e, "SupplementalProperty")?,
        audio_channel_configurations: descriptors(e, "AudioChannelConfiguration")?,
    })
}

fn conditional(name: &'static str, e: &Elem) -> Result<bool, MpdError> {
    match e.attr(name) {
        None => Ok(false),
        Some(v) => match v.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            digits => digits
                .parse::<u64>()
                .map(|n| n != 0)
                .map_err(|_| invalid(name, digits)),
        },
    }
}

fn adaptation_set(e: &Elem, ctx: &mut Ctx<'_>) -> Result<Option<AdaptationSet>, MpdError> {
    let link = xlink(e)?;
    if is_resolve_to_zero(&link) {
        ctx.features.xlink_resolve_to_zero = true;
        return Ok(None);
    }
    let start_with_sap = opt_u32(e, "startWithSAP")?
        .map(|v| u8::try_from(v).ok().filter(|v| *v <= MAX_START_WITH_SAP))
        .map(|v| v.ok_or_else(|| invalid("startWithSAP", "out of range")))
        .transpose()?;
    let mut representations = Vec::new();
    for r in e.children_named("Representation") {
        check_count(
            representations.len() + 1,
            ctx.limits.max_representations,
            Limit::Representations,
        )?;
        representations.push(representation(r, ctx)?);
    }
    Ok(Some(AdaptationSet {
        id: opt_u64(e, "id")?,
        group: opt_u64(e, "group")?,
        lang: opt_str(e, "lang"),
        content_type: opt_str(e, "contentType"),
        par: opt_str(e, "par"),
        segment_alignment: conditional("segmentAlignment", e)?,
        start_with_sap,
        bitstream_switching: opt_bool(e, "bitstreamSwitching")?,
        common: common(e)?,
        xlink: link,
        roles: descriptors(e, "Role")?,
        accessibilities: descriptors(e, "Accessibility")?,
        labels: e
            .children_named("Label")
            .map(|l| l.text.trim().to_owned())
            .collect(),
        base_urls: base_urls(e, ctx)?,
        segment: segment_layer(e, ctx)?,
        content_protections: descriptors(e, "ContentProtection")?,
        essential_properties: descriptors(e, "EssentialProperty")?,
        supplemental_properties: descriptors(e, "SupplementalProperty")?,
        audio_channel_configurations: descriptors(e, "AudioChannelConfiguration")?,
        representations,
    }))
}

fn period(e: &Elem, ctx: &mut Ctx<'_>) -> Result<Option<Period>, MpdError> {
    let link = xlink(e)?;
    if is_resolve_to_zero(&link) {
        ctx.features.xlink_resolve_to_zero = true;
        return Ok(None);
    }
    let mut sets = Vec::new();
    for a in e.children_named("AdaptationSet") {
        check_count(
            sets.len() + 1,
            ctx.limits.max_adaptation_sets,
            Limit::AdaptationSets,
        )?;
        if let Some(set) = adaptation_set(a, ctx)? {
            sets.push(set);
        }
    }
    Ok(Some(Period {
        id: opt_str(e, "id"),
        start: opt_duration(e, "start")?,
        duration: opt_duration(e, "duration")?,
        bitstream_switching: opt_bool(e, "bitstreamSwitching")?.unwrap_or(false),
        xlink: link,
        base_urls: base_urls(e, ctx)?,
        segment: segment_layer(e, ctx)?,
        asset_identifier: e.child("AssetIdentifier").map(descriptor).transpose()?,
        essential_properties: descriptors(e, "EssentialProperty")?,
        supplemental_properties: descriptors(e, "SupplementalProperty")?,
        adaptation_sets: sets,
    }))
}

fn scan_features(e: &Elem, f: &mut MpdFeatures) {
    if e.ns == Ns::Mpd {
        match e.name.as_str() {
            "PatchLocation" => f.patch_location = true,
            "EventStream" => f.event_stream = true,
            "InbandEventStream" => f.inband_event_stream = true,
            "Preselection" => f.preselection = true,
            "ServiceDescription" => f.service_description = true,
            "ProducerReferenceTime" => f.producer_reference_time = true,
            "ContentSteering" => f.content_steering = true,
            "Resync" => f.resync = true,
            _ => {}
        }
    }
    for c in &e.children {
        scan_features(c, f);
    }
}

impl Mpd {
    /// Parse an MPD document with the default [`Limits`].
    ///
    /// # Errors
    /// [`MpdError`] for malformed XML, a non-MPD root, a schema violation the
    /// core relies on, an exceeded bound, or an out-of-scope presentation type.
    pub fn parse(bytes: &[u8]) -> Result<Self, MpdError> {
        Self::parse_with(bytes, &Limits::default())
    }

    /// Parse an MPD document under explicit [`Limits`].
    ///
    /// # Errors
    /// See [`Mpd::parse`].
    pub fn parse_with(bytes: &[u8], limits: &Limits) -> Result<Self, MpdError> {
        let root = read_tree(bytes, limits)?;
        let mut ctx = Ctx {
            limits,
            features: MpdFeatures::default(),
        };
        let kind = match root.attr("type") {
            None | Some("static") => PresentationType::Static,
            Some("dynamic") => PresentationType::Dynamic,
            Some("list") => return Err(MpdError::Unsupported(Unsupported::PresentationTypeList)),
            Some(other) => return Err(invalid("MPD@type", other)),
        };
        let mut periods = Vec::new();
        for p in root.children_named("Period") {
            check_count(periods.len() + 1, limits.max_periods, Limit::Periods)?;
            if let Some(period) = period(p, &mut ctx)? {
                periods.push(period);
            }
        }
        let mut utc_timings = Vec::new();
        for u in root.children_named("UTCTiming") {
            let scheme_id_uri = u
                .attr("schemeIdUri")
                .ok_or_else(|| invalid("schemeIdUri", "missing on UTCTiming"))?
                .to_owned();
            utc_timings.push(UtcTiming {
                scheme: UtcScheme::from_scheme_id_uri(&scheme_id_uri),
                scheme_id_uri,
                value: u.attr("value").unwrap_or_default().to_owned(),
                id: opt_str(u, "id"),
            });
        }
        let base = base_urls(&root, &mut ctx)?;
        let mut features = ctx.features;
        scan_features(&root, &mut features);
        Ok(Self {
            id: opt_str(&root, "id"),
            profiles: root
                .attr("profiles")
                .map(|p| {
                    p.split(PROFILE_SEPARATOR)
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            kind,
            availability_start_time_us: opt_time(&root, "availabilityStartTime")?,
            availability_end_time_us: opt_time(&root, "availabilityEndTime")?,
            publish_time_us: opt_time(&root, "publishTime")?,
            media_presentation_duration: opt_duration(&root, "mediaPresentationDuration")?,
            minimum_update_period: opt_duration(&root, "minimumUpdatePeriod")?,
            min_buffer_time: opt_duration(&root, "minBufferTime")?,
            time_shift_buffer_depth: opt_duration(&root, "timeShiftBufferDepth")?,
            suggested_presentation_delay: opt_duration(&root, "suggestedPresentationDelay")?,
            max_segment_duration: opt_duration(&root, "maxSegmentDuration")?,
            max_subsegment_duration: opt_duration(&root, "maxSubsegmentDuration")?,
            base_urls: base,
            locations: root
                .children_named("Location")
                .map(|l| l.text.trim().to_owned())
                .collect(),
            utc_timings,
            essential_properties: descriptors(&root, "EssentialProperty")?,
            supplemental_properties: descriptors(&root, "SupplementalProperty")?,
            periods,
            features,
        })
    }
}
