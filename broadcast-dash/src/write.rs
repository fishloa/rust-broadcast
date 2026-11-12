//! Serialise the MPD model back to XML (ISO/IEC 23009-1).

use std::io;

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use crate::duration::format_iso8601_duration;
use crate::error::MpdError;
use crate::model::*;
use crate::parse::{INFINITY_TOKEN, NEGATIVE_INFINITY_TOKEN};
use crate::xs_datetime::format_xs_datetime;

const NS_MPD: &str = "urn:mpeg:dash:schema:mpd:2011";
const NS_DVB: &str = "urn:dvb:dash:dash-extensions:2014-1";
const NS_CENC: &str = "urn:mpeg:cenc:2013";
const NS_XLINK: &str = "http://www.w3.org/1999/xlink";
const NS_URLPARAM: &str = "urn:mpeg:dash:schema:urlparam:2014";

impl Mpd {
    /// Render the model as an MPD XML document.
    ///
    /// # Errors
    /// [`MpdError::Write`] if the in-memory writer fails (it cannot in
    /// practice) and [`MpdError::Invalid`] if a stored instant cannot be
    /// formatted.
    pub fn to_xml(&self) -> Result<String, MpdError> {
        let mut w = Writer::new_with_indent(Vec::new(), b' ', 2);
        let _ = w.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));
        let mut use_cenc = false;
        let mut use_xlink = false;
        let mut use_dvb = false;
        let mut use_urlparam = false;
        let mut scan_desc = |d: &Descriptor| {
            if d.default_kid.is_some() || d.pssh.is_some() {
                use_cenc = true;
            }
            if d.url_query.is_some() {
                use_urlparam = true;
            }
        };
        for d in &self.essential_properties {
            scan_desc(d);
        }
        for d in &self.supplemental_properties {
            scan_desc(d);
        }
        for p in &self.periods {
            if p.xlink.is_some() {
                use_xlink = true;
            }
            for es in &p.event_streams {
                if es.xlink.is_some() {
                    use_xlink = true;
                }
                for b in &es.base_urls {
                    if b.priority.is_some() || b.weight.is_some() {
                        use_dvb = true;
                    }
                }
                for d in es
                    .essential_properties
                    .iter()
                    .chain(es.supplemental_properties.iter())
                    .chain(es.events.iter().flat_map(|e| {
                        e.essential_properties
                            .iter()
                            .chain(e.supplemental_properties.iter())
                    }))
                {
                    scan_desc(d);
                }
            }
            for pre in &p.preselections {
                for d in pre
                    .audio_channel_configurations
                    .iter()
                    .chain(pre.essential_properties.iter())
                    .chain(pre.supplemental_properties.iter())
                    .chain(pre.accessibilities.iter())
                    .chain(pre.roles.iter())
                    .chain(pre.ratings.iter())
                    .chain(pre.viewpoints.iter())
                {
                    scan_desc(d);
                }
            }
            for b in &p.base_urls {
                if b.priority.is_some() || b.weight.is_some() {
                    use_dvb = true;
                }
            }
            for a in &p.adaptation_sets {
                if a.xlink.is_some() {
                    use_xlink = true;
                }
                for b in &a.base_urls {
                    if b.priority.is_some() || b.weight.is_some() {
                        use_dvb = true;
                    }
                }
                for r in &a.representations {
                    for b in &r.base_urls {
                        if b.priority.is_some() || b.weight.is_some() {
                            use_dvb = true;
                        }
                    }
                }
                for d in a
                    .roles
                    .iter()
                    .chain(a.accessibilities.iter())
                    .chain(a.content_protections.iter())
                    .chain(a.essential_properties.iter())
                    .chain(a.supplemental_properties.iter())
                    .chain(a.audio_channel_configurations.iter())
                    .chain(a.inband_event_streams.iter())
                {
                    scan_desc(d);
                }
                for r in &a.representations {
                    for d in r
                        .content_protections
                        .iter()
                        .chain(r.essential_properties.iter())
                        .chain(r.supplemental_properties.iter())
                        .chain(r.audio_channel_configurations.iter())
                        .chain(r.inband_event_streams.iter())
                    {
                        scan_desc(d);
                    }
                }
            }
            for b in &p.base_urls {
                if b.priority.is_some() || b.weight.is_some() {
                    use_dvb = true;
                }
            }
            if p.xlink.is_some() {
                use_xlink = true;
            }
        }
        for b in &self.base_urls {
            if b.priority.is_some() || b.weight.is_some() {
                use_dvb = true;
            }
        }

        let mut root = BytesStart::new("MPD");
        root.push_attribute(("xmlns", NS_MPD));
        if use_dvb {
            root.push_attribute(("xmlns:dvb", NS_DVB));
        }
        if use_xlink {
            root.push_attribute(("xmlns:xlink", NS_XLINK));
        }
        if use_urlparam {
            root.push_attribute(("xmlns:urlparam", NS_URLPARAM));
        }
        if !self.profiles.is_empty() {
            root.push_attribute(("profiles", self.profiles.join(",").as_str()));
        }
        root.push_attribute(("type", self.kind.name()));
        if let Some(min_buffer) = self.min_buffer_time.map(format_iso8601_duration) {
            root.push_attribute(("minBufferTime", min_buffer.as_str()));
        }
        let cenc_needed = use_cenc;
        if cenc_needed {
            root.push_attribute(("xmlns:cenc", NS_CENC));
        }
        if let Some(v) = self.availability_start_time_us {
            root.push_attribute((
                "availabilityStartTime",
                format_xs_datetime(v)
                    .map_err(|e| MpdError::Invalid {
                        what: "availabilityStartTime",
                        detail: e.to_string(),
                    })?
                    .as_str(),
            ));
        }
        if let Some(v) = self.publish_time_us {
            root.push_attribute((
                "publishTime",
                format_xs_datetime(v)
                    .map_err(|e| MpdError::Invalid {
                        what: "publishTime",
                        detail: e.to_string(),
                    })?
                    .as_str(),
            ));
        }
        if let Some(v) = self.minimum_update_period {
            root.push_attribute(("minimumUpdatePeriod", format_iso8601_duration(v).as_str()));
        }
        if let Some(v) = self.time_shift_buffer_depth {
            root.push_attribute(("timeShiftBufferDepth", format_iso8601_duration(v).as_str()));
        }
        if let Some(v) = self.suggested_presentation_delay {
            root.push_attribute((
                "suggestedPresentationDelay",
                format_iso8601_duration(v).as_str(),
            ));
        }
        if let Some(v) = self.media_presentation_duration {
            root.push_attribute((
                "mediaPresentationDuration",
                format_iso8601_duration(v).as_str(),
            ));
        }
        if let Some(v) = &self.id {
            root.push_attribute(("id", v.as_str()));
        }
        if let Some(v) = self.availability_end_time_us {
            root.push_attribute((
                "availabilityEndTime",
                format_xs_datetime(v)
                    .map_err(|e| MpdError::Invalid {
                        what: "availabilityEndTime",
                        detail: e.to_string(),
                    })?
                    .as_str(),
            ));
        }
        if let Some(v) = self.max_segment_duration {
            root.push_attribute(("maxSegmentDuration", format_iso8601_duration(v).as_str()));
        }
        if let Some(v) = self.max_subsegment_duration {
            root.push_attribute(("maxSubsegmentDuration", format_iso8601_duration(v).as_str()));
        }
        w.write_event(Event::Start(root)).map_err(io_err)?;
        for b in &self.base_urls {
            write_base_url(&mut w, b)?;
        }
        for l in &self.locations {
            w.write_event(Event::Start(BytesStart::new("Location")))
                .map_err(io_err)?;
            w.write_event(Event::Text(BytesText::new(l)))
                .map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("Location")))
                .map_err(io_err)?;
        }
        for pl in &self.patch_locations {
            write_patch_location(&mut w, pl)?;
        }
        for sd in &self.service_descriptions {
            write_service_description(&mut w, sd)?;
        }
        for p in &self.periods {
            write_period(&mut w, p)?;
        }
        for d in &self.essential_properties {
            write_descriptor(&mut w, "EssentialProperty", d)?;
        }
        for d in &self.supplemental_properties {
            write_descriptor(&mut w, "SupplementalProperty", d)?;
        }
        for u in &self.utc_timings {
            write_utc_timing(&mut w, u)?;
        }
        if let Some(cs) = &self.content_steering {
            write_content_steering(&mut w, cs)?;
        }
        w.write_event(Event::End(BytesEnd::new("MPD")))
            .map_err(io_err)?;
        let mut out = String::from_utf8_lossy(&w.into_inner()).into_owned();
        out.push('\n');
        Ok(out)
    }
}

fn write_text_element(
    w: &mut Writer<Vec<u8>>,
    e: BytesStart<'_>,
    name: &str,
    text: &str,
) -> Result<(), MpdError> {
    if text.is_empty() {
        return w.write_event(Event::Empty(e)).map_err(io_err);
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(name)))
        .map_err(io_err)
}

fn write_utc_timing(w: &mut Writer<Vec<u8>>, u: &UtcTiming) -> Result<(), MpdError> {
    let mut e = BytesStart::new("UTCTiming");
    e.push_attribute(("schemeIdUri", u.scheme_id_uri.as_str()));
    e.push_attribute(("value", u.value.as_str()));
    if let Some(v) = &u.id {
        e.push_attribute(("id", v.as_str()));
    }
    w.write_event(Event::Empty(e)).map_err(io_err)
}

fn write_patch_location(w: &mut Writer<Vec<u8>>, p: &PatchLocation) -> Result<(), MpdError> {
    let mut e = BytesStart::new("PatchLocation");
    if let Some(v) = &p.service_location {
        e.push_attribute(("serviceLocation", v.as_str()));
    }
    if let Some(v) = p.ttl {
        e.push_attribute(("ttl", format_xs_double(v).as_str()));
    }
    write_text_element(w, e, "PatchLocation", &p.url)
}

fn write_content_steering(w: &mut Writer<Vec<u8>>, c: &ContentSteering) -> Result<(), MpdError> {
    let mut e = BytesStart::new("ContentSteering");
    if let Some(v) = &c.default_service_location {
        e.push_attribute(("defaultServiceLocation", v.as_str()));
    }
    if c.query_before_start {
        e.push_attribute(("queryBeforeStart", "true"));
    }
    if let Some(v) = c.client_requirement {
        e.push_attribute(("clientRequirement", if v { "true" } else { "false" }));
    }
    write_text_element(w, e, "ContentSteering", &c.url)
}

fn write_event(w: &mut Writer<Vec<u8>>, ev: &StreamEvent) -> Result<(), MpdError> {
    if let Some(u) = ev.unsupported {
        return Err(MpdError::Unsupported(u));
    }
    let mut e = BytesStart::new("Event");
    e.push_attribute((
        "presentationTime",
        ev.presentation_time.to_string().as_str(),
    ));
    if let Some(v) = ev.duration {
        e.push_attribute(("duration", v.to_string().as_str()));
    }
    if let Some(v) = ev.id {
        e.push_attribute(("id", v.to_string().as_str()));
    }
    if let Some(v) = ev.status {
        e.push_attribute(("status", v.name()));
    }
    if let Some(v) = &ev.message_data {
        e.push_attribute(("messageData", v.as_str()));
    }
    let has_children = !(ev.supplemental_properties.is_empty()
        && ev.essential_properties.is_empty()
        && ev.service_descriptions.is_empty());
    if !has_children {
        return write_text_element(w, e, "Event", &ev.text);
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    if !ev.text.is_empty() {
        w.write_event(Event::Text(BytesText::new(&ev.text)))
            .map_err(io_err)?;
    }
    for sd in &ev.service_descriptions {
        write_service_description(w, sd)?;
    }
    for d in &ev.supplemental_properties {
        write_descriptor(w, "SupplementalProperty", d)?;
    }
    for d in &ev.essential_properties {
        write_descriptor(w, "EssentialProperty", d)?;
    }
    w.write_event(Event::End(BytesEnd::new("Event")))
        .map_err(io_err)
}

fn write_event_stream(w: &mut Writer<Vec<u8>>, es: &EventStream) -> Result<(), MpdError> {
    let mut e = BytesStart::new("EventStream");
    if let Some(x) = &es.xlink {
        write_xlink_attrs(&mut e, x);
    }
    e.push_attribute(("schemeIdUri", es.scheme_id_uri.as_str()));
    if let Some(v) = &es.value {
        e.push_attribute(("value", v.as_str()));
    }
    if let Some(v) = es.timescale {
        e.push_attribute(("timescale", v.to_string().as_str()));
    }
    if let Some(v) = es.presentation_time_offset {
        e.push_attribute(("presentationTimeOffset", v.to_string().as_str()));
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    for ev in &es.events {
        write_event(w, ev)?;
    }
    for b in &es.base_urls {
        write_base_url(w, b)?;
    }
    for r in &es.request_params {
        write_request_param(w, r)?;
    }
    for d in &es.essential_properties {
        write_descriptor(w, "EssentialProperty", d)?;
    }
    for d in &es.supplemental_properties {
        write_descriptor(w, "SupplementalProperty", d)?;
    }
    w.write_event(Event::End(BytesEnd::new("EventStream")))
        .map_err(io_err)
}

fn write_producer_reference_time(
    w: &mut Writer<Vec<u8>>,
    p: &ProducerReferenceTime,
) -> Result<(), MpdError> {
    let mut e = BytesStart::new("ProducerReferenceTime");
    e.push_attribute(("id", p.id.to_string().as_str()));
    if p.inband {
        e.push_attribute(("inband", "true"));
    }
    if p.kind != ProducerReferenceKind::Encoder {
        e.push_attribute(("type", p.kind.name()));
    }
    if let Some(v) = &p.application_scheme {
        e.push_attribute(("applicationScheme", v.as_str()));
    }
    e.push_attribute(("wallClockTime", p.wall_clock_time.as_str()));
    e.push_attribute(("presentationTime", p.presentation_time.to_string().as_str()));
    match &p.utc_timing {
        None => w.write_event(Event::Empty(e)).map_err(io_err),
        Some(u) => {
            w.write_event(Event::Start(e)).map_err(io_err)?;
            write_utc_timing(w, u)?;
            w.write_event(Event::End(BytesEnd::new("ProducerReferenceTime")))
                .map_err(io_err)
        }
    }
}

fn write_resync(w: &mut Writer<Vec<u8>>, r: &Resync) -> Result<(), MpdError> {
    let mut e = BytesStart::new("Resync");
    if r.kind != 0 {
        e.push_attribute(("type", r.kind.to_string().as_str()));
    }
    if let Some(v) = r.dt {
        e.push_attribute(("dT", v.to_string().as_str()));
    }
    if let Some(v) = r.d_imax {
        e.push_attribute(("dImax", format_xs_double(v).as_str()));
    }
    if let Some(v) = r.d_imin {
        e.push_attribute(("dImin", format_xs_double(v).as_str()));
    }
    if r.marker {
        e.push_attribute(("marker", "true"));
    }
    w.write_event(Event::Empty(e)).map_err(io_err)
}

fn write_label(w: &mut Writer<Vec<u8>>, name: &str, text: &str) -> Result<(), MpdError> {
    w.write_event(Event::Start(BytesStart::new(name)))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(name)))
        .map_err(io_err)
}

fn write_preselection(w: &mut Writer<Vec<u8>>, p: &Preselection) -> Result<(), MpdError> {
    if let Some(u) = p.unsupported {
        return Err(MpdError::Unsupported(u));
    }
    let mut e = BytesStart::new("Preselection");
    if let Some(v) = &p.id {
        e.push_attribute(("id", v.as_str()));
    }
    e.push_attribute((
        "preselectionComponents",
        p.preselection_components.join(" ").as_str(),
    ));
    if let Some(v) = &p.lang {
        e.push_attribute(("lang", v.as_str()));
    }
    if let Some(v) = p.order {
        e.push_attribute(("order", v.name()));
    }
    if let Some(v) = &p.common.profiles {
        e.push_attribute(("profiles", v.as_str()));
    }
    if let Some(v) = p.common.width {
        e.push_attribute(("width", v.to_string().as_str()));
    }
    if let Some(v) = p.common.height {
        e.push_attribute(("height", v.to_string().as_str()));
    }
    if let Some(v) = &p.common.sar {
        e.push_attribute(("sar", v.as_str()));
    }
    if let Some(v) = &p.common.frame_rate {
        e.push_attribute(("frameRate", v.as_str()));
    }
    if let Some(v) = &p.common.audio_sampling_rate {
        e.push_attribute(("audioSamplingRate", v.as_str()));
    }
    if let Some(v) = &p.common.mime_type {
        e.push_attribute(("mimeType", v.as_str()));
    }
    if let Some(v) = &p.common.codecs {
        e.push_attribute(("codecs", v.as_str()));
    }
    if let Some(v) = p.common.max_playout_rate {
        e.push_attribute(("maxPlayoutRate", format_xs_double(v).as_str()));
    }
    if let Some(v) = p.common.coding_dependency {
        e.push_attribute(("codingDependency", if v { "true" } else { "false" }));
    }
    if let Some(v) = &p.segment_profiles {
        e.push_attribute(("segmentProfiles", v.as_str()));
    }
    if let Some(v) = &p.container_profiles {
        e.push_attribute(("containerProfiles", v.as_str()));
    }
    if let Some(v) = p.maximum_sap_period {
        e.push_attribute(("maximumSAPPeriod", format_xs_double(v).as_str()));
    }
    if let Some(v) = p.start_with_sap {
        e.push_attribute(("startWithSAP", v.to_string().as_str()));
    }
    if let Some(v) = &p.scan_type {
        e.push_attribute(("scanType", v.as_str()));
    }
    if let Some(v) = p.selection_priority {
        e.push_attribute(("selectionPriority", v.to_string().as_str()));
    }
    if let Some(v) = &p.tag {
        e.push_attribute(("tag", v.as_str()));
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    for d in &p.frame_packings {
        write_descriptor(w, "FramePacking", d)?;
    }
    for d in &p.audio_channel_configurations {
        write_descriptor(w, "AudioChannelConfiguration", d)?;
    }
    for d in &p.content_protections {
        write_descriptor(w, "ContentProtection", d)?;
    }
    if let Some(d) = &p.output_protection {
        write_descriptor(w, "OutputProtection", d)?;
    }
    for d in &p.essential_properties {
        write_descriptor(w, "EssentialProperty", d)?;
    }
    for d in &p.supplemental_properties {
        write_descriptor(w, "SupplementalProperty", d)?;
    }
    for d in &p.inband_event_streams {
        write_descriptor(w, "InbandEventStream", d)?;
    }
    for l in &p.group_labels {
        write_label(w, "GroupLabel", l)?;
    }
    for l in &p.labels {
        write_label(w, "Label", l)?;
    }
    for t in &p.producer_reference_times {
        write_producer_reference_time(w, t)?;
    }
    for r in &p.resyncs {
        write_resync(w, r)?;
    }
    for d in &p.accessibilities {
        write_descriptor(w, "Accessibility", d)?;
    }
    for d in &p.roles {
        write_descriptor(w, "Role", d)?;
    }
    for d in &p.ratings {
        write_descriptor(w, "Rating", d)?;
    }
    for d in &p.viewpoints {
        write_descriptor(w, "Viewpoint", d)?;
    }
    w.write_event(Event::End(BytesEnd::new("Preselection")))
        .map_err(io_err)
}

fn write_service_description(
    w: &mut Writer<Vec<u8>>,
    sd: &ServiceDescription,
) -> Result<(), MpdError> {
    let mut e = BytesStart::new("ServiceDescription");
    if let Some(v) = sd.id {
        e.push_attribute(("id", v.to_string().as_str()));
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    if let Some(lat) = &sd.latency {
        let mut le = BytesStart::new("Latency");
        if let Some(v) = lat.target {
            le.push_attribute(("target", v.to_string().as_str()));
        }
        if let Some(v) = lat.min {
            le.push_attribute(("min", v.to_string().as_str()));
        }
        if let Some(v) = lat.max {
            le.push_attribute(("max", v.to_string().as_str()));
        }
        if let Some(v) = lat.reference_id {
            le.push_attribute(("referenceId", v.to_string().as_str()));
        }
        w.write_event(Event::Empty(le)).map_err(io_err)?;
    }
    if let Some(pr) = &sd.playback_rate {
        let mut pe = BytesStart::new("PlaybackRate");
        if let Some(v) = &pr.min {
            pe.push_attribute(("min", v.as_str()));
        }
        if let Some(v) = &pr.max {
            pe.push_attribute(("max", v.as_str()));
        }
        w.write_event(Event::Empty(pe)).map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("ServiceDescription")))
        .map_err(io_err)?;
    Ok(())
}

fn io_err(e: io::Error) -> MpdError {
    MpdError::Write {
        message: e.to_string(),
    }
}

fn write_base_url(w: &mut Writer<Vec<u8>>, b: &BaseUrl) -> Result<(), MpdError> {
    let mut e = BytesStart::new("BaseURL");
    if let Some(v) = &b.service_location {
        e.push_attribute(("serviceLocation", v.as_str()));
    }
    if let Some(v) = b.priority {
        e.push_attribute(("dvb:priority", v.to_string().as_str()));
    }
    if let Some(v) = b.weight {
        e.push_attribute(("dvb:weight", v.to_string().as_str()));
    }
    if let Some(v) = b.availability_time_offset {
        e.push_attribute(("availabilityTimeOffset", format_xs_double(v).as_str()));
    }
    if let Some(v) = b.availability_time_complete {
        e.push_attribute(("availabilityTimeComplete", if v { "true" } else { "false" }));
    }
    if let Some(v) = b.time_shift_buffer_depth {
        e.push_attribute(("timeShiftBufferDepth", format_iso8601_duration(v).as_str()));
    }
    if b.url.is_empty() && e.attributes().count() == 0 {
        w.write_event(Event::Empty(e)).map_err(io_err)?;
    } else {
        w.write_event(Event::Start(e)).map_err(io_err)?;
        if !b.url.is_empty() {
            w.write_event(Event::Text(BytesText::new(&b.url)))
                .map_err(io_err)?;
        }
        w.write_event(Event::End(BytesEnd::new("BaseURL")))
            .map_err(io_err)?;
    }
    Ok(())
}

fn format_xs_double(v: f64) -> String {
    if v == f64::INFINITY {
        INFINITY_TOKEN.to_string()
    } else if v == f64::NEG_INFINITY {
        NEGATIVE_INFINITY_TOKEN.to_string()
    } else {
        v.to_string()
    }
}

fn write_range(r: &ByteRange) -> String {
    match r.last {
        Some(l) => format!("{}-{}", r.first, l),
        None => format!("{}-", r.first),
    }
}

fn write_url_type(w: &mut Writer<Vec<u8>>, name: &str, u: &UrlType) -> Result<(), MpdError> {
    let mut e = BytesStart::new(name);
    if let Some(v) = &u.source_url {
        e.push_attribute(("sourceURL", v.as_str()));
    }
    if let Some(v) = &u.range {
        e.push_attribute(("range", write_range(v).as_str()));
    }
    w.write_event(Event::Empty(e)).map_err(io_err)?;
    Ok(())
}

fn write_base_info(
    _w: &mut Writer<Vec<u8>>,
    e: &mut BytesStart<'_>,
    info: &BaseInfo,
) -> Result<(), MpdError> {
    if let Some(v) = info.timescale {
        e.push_attribute(("timescale", v.to_string().as_str()));
    }
    if let Some(v) = info.presentation_time_offset {
        e.push_attribute(("presentationTimeOffset", v.to_string().as_str()));
    }
    if let Some(v) = info.ept_delta {
        e.push_attribute(("eptDelta", v.to_string().as_str()));
    }
    if let Some(v) = info.pd_delta {
        e.push_attribute(("pdDelta", v.to_string().as_str()));
    }
    if let Some(v) = info.presentation_duration {
        e.push_attribute(("presentationDuration", v.to_string().as_str()));
    }
    if let Some(v) = info.time_shift_buffer_depth {
        e.push_attribute(("timeShiftBufferDepth", format_iso8601_duration(v).as_str()));
    }
    if let Some(v) = &info.index_range {
        e.push_attribute(("indexRange", write_range(v).as_str()));
    }
    if let Some(v) = info.index_range_exact {
        e.push_attribute(("indexRangeExact", if v { "true" } else { "false" }));
    }
    Ok(())
}

fn write_segment_base(w: &mut Writer<Vec<u8>>, sb: &SegmentBase) -> Result<(), MpdError> {
    let mut e = BytesStart::new("SegmentBase");
    write_base_info(w, &mut e, &sb.info)?;
    if let Some(v) = sb.info.availability_time_offset {
        e.push_attribute(("availabilityTimeOffset", format_xs_double(v).as_str()));
    }
    if let Some(v) = sb.info.availability_time_complete {
        e.push_attribute(("availabilityTimeComplete", if v { "true" } else { "false" }));
    }
    let has_init = sb.info.initialization.is_some();
    let has_ri = sb.info.representation_index.is_some();
    if !has_init && !has_ri {
        w.write_event(Event::Empty(e)).map_err(io_err)?;
        return Ok(());
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    if let Some(u) = &sb.info.initialization {
        write_url_type(w, "Initialization", u)?;
    }
    if let Some(u) = &sb.info.representation_index {
        write_url_type(w, "RepresentationIndex", u)?;
    }
    w.write_event(Event::End(BytesEnd::new("SegmentBase")))
        .map_err(io_err)?;
    Ok(())
}

fn write_timeline_entry(w: &mut Writer<Vec<u8>>, te: &TimelineEntry) -> Result<(), MpdError> {
    let mut e = BytesStart::new("S");
    if let Some(v) = te.t {
        e.push_attribute(("t", v.to_string().as_str()));
    }
    if let Some(v) = te.n {
        e.push_attribute(("n", v.to_string().as_str()));
    }
    e.push_attribute(("d", te.d.to_string().as_str()));
    if te.r != 0 {
        e.push_attribute(("r", te.r.to_string().as_str()));
    }
    if let Some(v) = te.k {
        e.push_attribute(("k", v.to_string().as_str()));
    }
    w.write_event(Event::Empty(e)).map_err(io_err)?;
    Ok(())
}

fn write_timeline(w: &mut Writer<Vec<u8>>, t: &SegmentTimeline) -> Result<(), MpdError> {
    let e = BytesStart::new("SegmentTimeline");
    w.write_event(Event::Start(e)).map_err(io_err)?;
    for te in &t.entries {
        write_timeline_entry(w, te)?;
    }
    w.write_event(Event::End(BytesEnd::new("SegmentTimeline")))
        .map_err(io_err)?;
    Ok(())
}

fn write_segment_list(w: &mut Writer<Vec<u8>>, sl: &SegmentList) -> Result<(), MpdError> {
    let mut e = BytesStart::new("SegmentList");
    write_base_info(w, &mut e, &sl.multi.base)?;
    if let Some(v) = sl.multi.base.availability_time_offset {
        e.push_attribute(("availabilityTimeOffset", format_xs_double(v).as_str()));
    }
    if let Some(v) = sl.multi.base.availability_time_complete {
        e.push_attribute(("availabilityTimeComplete", if v { "true" } else { "false" }));
    }
    if let Some(v) = sl.multi.duration {
        e.push_attribute(("duration", v.to_string().as_str()));
    }
    if let Some(v) = sl.multi.start_number {
        e.push_attribute(("startNumber", v.to_string().as_str()));
    }
    if let Some(v) = sl.multi.end_number {
        e.push_attribute(("endNumber", v.to_string().as_str()));
    }
    if let Some(x) = &sl.xlink {
        e.push_attribute(("xlink:href", x.href.as_str()));
        if x.actuate != Actuate::OnRequest {
            e.push_attribute(("xlink:actuate", x.actuate.name()));
        }
    }
    let has_children = sl.multi.base.initialization.is_some()
        || sl.multi.base.representation_index.is_some()
        || sl.multi.timeline.is_some()
        || sl.multi.bitstream_switching.is_some()
        || !sl.urls.is_empty();
    if !has_children {
        w.write_event(Event::Empty(e)).map_err(io_err)?;
        return Ok(());
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    if let Some(u) = &sl.multi.base.initialization {
        write_url_type(w, "Initialization", u)?;
    }
    if let Some(u) = &sl.multi.base.representation_index {
        write_url_type(w, "RepresentationIndex", u)?;
    }
    if let Some(t) = &sl.multi.timeline {
        write_timeline(w, t)?;
    }
    if let Some(u) = &sl.multi.bitstream_switching {
        write_url_type(w, "BitstreamSwitching", u)?;
    }
    for u in &sl.urls {
        write_segment_url(w, u)?;
    }
    w.write_event(Event::End(BytesEnd::new("SegmentList")))
        .map_err(io_err)?;
    Ok(())
}

fn write_segment_url(w: &mut Writer<Vec<u8>>, u: &SegmentUrl) -> Result<(), MpdError> {
    let mut e = BytesStart::new("SegmentURL");
    if let Some(v) = &u.media {
        e.push_attribute(("media", v.as_str()));
    }
    if let Some(v) = &u.media_range {
        e.push_attribute(("mediaRange", write_range(v).as_str()));
    }
    if let Some(v) = &u.index {
        e.push_attribute(("index", v.as_str()));
    }
    if let Some(v) = &u.index_range {
        e.push_attribute(("indexRange", write_range(v).as_str()));
    }
    w.write_event(Event::Empty(e)).map_err(io_err)?;
    Ok(())
}

fn write_segment_template(w: &mut Writer<Vec<u8>>, st: &SegmentTemplate) -> Result<(), MpdError> {
    let mut e = BytesStart::new("SegmentTemplate");
    write_base_info(w, &mut e, &st.multi.base)?;
    if let Some(v) = st.multi.duration {
        e.push_attribute(("duration", v.to_string().as_str()));
    }
    if let Some(v) = st.multi.start_number {
        e.push_attribute(("startNumber", v.to_string().as_str()));
    }
    if let Some(v) = st.multi.end_number {
        e.push_attribute(("endNumber", v.to_string().as_str()));
    }
    if let Some(v) = &st.initialization {
        e.push_attribute(("initialization", v.as_str()));
    }
    if let Some(v) = &st.media {
        e.push_attribute(("media", v.as_str()));
    }
    if let Some(v) = &st.index {
        e.push_attribute(("index", v.as_str()));
    }
    if let Some(v) = &st.bitstream_switching {
        e.push_attribute(("bitstreamSwitching", v.as_str()));
    }
    if let Some(v) = st.multi.base.availability_time_offset {
        e.push_attribute(("availabilityTimeOffset", format_xs_double(v).as_str()));
    }
    if let Some(v) = st.multi.base.availability_time_complete {
        e.push_attribute(("availabilityTimeComplete", if v { "true" } else { "false" }));
    }
    let has_children = st.multi.base.initialization.is_some()
        || st.multi.base.representation_index.is_some()
        || st.multi.timeline.is_some()
        || st.multi.bitstream_switching.is_some();
    if !has_children {
        w.write_event(Event::Empty(e)).map_err(io_err)?;
        return Ok(());
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    if let Some(u) = &st.multi.base.initialization {
        write_url_type(w, "Initialization", u)?;
    }
    if let Some(u) = &st.multi.base.representation_index {
        write_url_type(w, "RepresentationIndex", u)?;
    }
    if let Some(t) = &st.multi.timeline {
        write_timeline(w, t)?;
    }
    if let Some(u) = &st.multi.bitstream_switching {
        write_url_type(w, "BitstreamSwitching", u)?;
    }
    w.write_event(Event::End(BytesEnd::new("SegmentTemplate")))
        .map_err(io_err)?;
    Ok(())
}

fn write_segment_layer(w: &mut Writer<Vec<u8>>, layer: &SegmentLayer) -> Result<(), MpdError> {
    if let Some(b) = &layer.base {
        write_segment_base(w, b)?;
    }
    if let Some(l) = &layer.list {
        write_segment_list(w, l)?;
    }
    if let Some(t) = &layer.template {
        write_segment_template(w, t)?;
    }
    Ok(())
}

fn write_descriptor(w: &mut Writer<Vec<u8>>, name: &str, d: &Descriptor) -> Result<(), MpdError> {
    let mut e = BytesStart::new(name);
    e.push_attribute(("schemeIdUri", d.scheme_id_uri.as_str()));
    if let Some(v) = &d.value {
        e.push_attribute(("value", v.as_str()));
    }
    if let Some(v) = &d.id {
        e.push_attribute(("id", v.as_str()));
    }
    if let Some(v) = &d.default_kid {
        e.push_attribute(("cenc:default_KID", v.as_str()));
    }
    let has_children = d.url_query.is_some() || d.pssh.is_some();
    if has_children {
        w.write_event(Event::Start(e)).map_err(io_err)?;
        if let Some(p) = &d.pssh {
            w.write_event(Event::Start(BytesStart::new("cenc:pssh")))
                .map_err(io_err)?;
            w.write_event(Event::Text(BytesText::new(p)))
                .map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("cenc:pssh")))
                .map_err(io_err)?;
        }
        if let Some(q) = &d.url_query {
            let mut qe = BytesStart::new("urlparam:UrlQueryInfo");
            qe.push_attribute((
                "useMPDUrlQuery",
                if q.use_mpd_url_query { "true" } else { "false" },
            ));
            push_query_attrs(&mut qe, q);
            w.write_event(Event::Empty(qe)).map_err(io_err)?;
        }
        w.write_event(Event::End(BytesEnd::new(name)))
            .map_err(io_err)?;
    } else {
        w.write_event(Event::Empty(e)).map_err(io_err)?;
    }
    Ok(())
}

fn push_query_attrs(e: &mut BytesStart<'_>, q: &UrlQueryInfo) {
    if let Some(v) = &q.query_string {
        e.push_attribute(("queryString", v.as_str()));
    }
    if let Some(v) = &q.query_template {
        e.push_attribute(("queryTemplate", v.as_str()));
    }
}

fn write_request_param(w: &mut Writer<Vec<u8>>, r: &RequestParam) -> Result<(), MpdError> {
    let mut e = BytesStart::new("RequestParam");
    if let Some(x) = &r.xlink {
        write_xlink_attrs(&mut e, x);
    }
    if r.query.use_mpd_url_query {
        e.push_attribute(("useMPDUrlQuery", "true"));
    }
    push_query_attrs(&mut e, &r.query);
    if let Some(v) = &r.include_in_requests {
        e.push_attribute(("includeInRequests", v.as_str()));
    }
    if let Some(v) = &r.header_param_source {
        e.push_attribute(("headerParamSource", v.as_str()));
    }
    if let Some(v) = r.same_origin_only {
        e.push_attribute(("sameOriginOnly", if v { "true" } else { "false" }));
    }
    if let Some(v) = &r.header {
        e.push_attribute(("header", v.as_str()));
    }
    w.write_event(Event::Empty(e)).map_err(io_err)
}

fn write_xlink_attrs(e: &mut BytesStart<'_>, x: &Xlink) {
    e.push_attribute(("xlink:href", x.href.as_str()));
    if x.actuate != Actuate::OnRequest {
        e.push_attribute(("xlink:actuate", x.actuate.name()));
    }
}

fn write_period(w: &mut Writer<Vec<u8>>, p: &Period) -> Result<(), MpdError> {
    let mut e = BytesStart::new("Period");
    if let Some(v) = &p.id {
        e.push_attribute(("id", v.as_str()));
    }
    if let Some(v) = p.start {
        e.push_attribute(("start", format_iso8601_duration(v).as_str()));
    }
    if let Some(v) = p.duration {
        e.push_attribute(("duration", format_iso8601_duration(v).as_str()));
    }
    if p.bitstream_switching {
        e.push_attribute(("bitstreamSwitching", "true"));
    }
    if let Some(x) = &p.xlink {
        write_xlink_attrs(&mut e, x);
    }
    w.write_event(Event::Start(e)).map_err(io_err)?;
    if let Some(im) = &p.imported_mpd {
        let mut ie = BytesStart::new("ImportedMPD");
        if let Some(v) = im.earliest_resolution_time_offset {
            ie.push_attribute(("earliestResolutionTimeOffset", v.to_string().as_str()));
        }
        w.write_event(Event::Start(ie)).map_err(io_err)?;
        w.write_event(Event::Text(BytesText::new(&im.url)))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("ImportedMPD")))
            .map_err(io_err)?;
    }
    for b in &p.base_urls {
        write_base_url(w, b)?;
    }
    write_segment_layer(w, &p.segment)?;
    if let Some(d) = &p.asset_identifier {
        write_descriptor(w, "AssetIdentifier", d)?;
    }
    for es in &p.event_streams {
        write_event_stream(w, es)?;
    }
    for sd in &p.service_descriptions {
        write_service_description(w, sd)?;
    }
    for a in &p.adaptation_sets {
        write_adaptation_set(w, a)?;
    }
    for d in &p.essential_properties {
        write_descriptor(w, "EssentialProperty", d)?;
    }
    for d in &p.supplemental_properties {
        write_descriptor(w, "SupplementalProperty", d)?;
    }
    for pre in &p.preselections {
        write_preselection(w, pre)?;
    }
    w.write_event(Event::End(BytesEnd::new("Period")))
        .map_err(io_err)?;
    Ok(())
}

fn write_adaptation_set(w: &mut Writer<Vec<u8>>, a: &AdaptationSet) -> Result<(), MpdError> {
    let mut e = BytesStart::new("AdaptationSet");
    if let Some(v) = a.id {
        e.push_attribute(("id", v.to_string().as_str()));
    }
    if let Some(v) = &a.content_type {
        e.push_attribute(("contentType", v.as_str()));
    }
    if let Some(v) = &a.common.mime_type {
        e.push_attribute(("mimeType", v.as_str()));
    }
    if let Some(v) = a.common.max_playout_rate {
        e.push_attribute(("maxPlayoutRate", v.to_string().as_str()));
    }
    if let Some(v) = a.common.coding_dependency {
        e.push_attribute(("codingDependency", if v { "true" } else { "false" }));
    }
    if a.segment_alignment {
        e.push_attribute(("segmentAlignment", "true"));
    }
    if let Some(v) = a.start_with_sap {
        e.push_attribute(("startWithSAP", v.to_string().as_str()));
    }
    if let Some(v) = &a.par {
        e.push_attribute(("par", v.as_str()));
    }
    if let Some(v) = &a.lang {
        e.push_attribute(("lang", v.as_str()));
    }
    if let Some(v) = a.group {
        e.push_attribute(("group", v.to_string().as_str()));
    }
    if let Some(v) = a.bitstream_switching {
        e.push_attribute(("bitstreamSwitching", if v { "true" } else { "false" }));
    }
    if let Some(v) = &a.common.profiles {
        e.push_attribute(("profiles", v.as_str()));
    }
    if let Some(v) = a.common.width {
        e.push_attribute(("width", v.to_string().as_str()));
    }
    if let Some(v) = a.common.height {
        e.push_attribute(("height", v.to_string().as_str()));
    }
    if let Some(v) = &a.common.sar {
        e.push_attribute(("sar", v.as_str()));
    }
    if let Some(v) = &a.common.frame_rate {
        e.push_attribute(("frameRate", v.as_str()));
    }
    if let Some(v) = &a.common.audio_sampling_rate {
        e.push_attribute(("audioSamplingRate", v.as_str()));
    }
    if let Some(v) = &a.common.codecs {
        e.push_attribute(("codecs", v.as_str()));
    }
    if let Some(x) = &a.xlink {
        write_xlink_attrs(&mut e, x);
    }

    w.write_event(Event::Start(e)).map_err(io_err)?;
    for d in &a.audio_channel_configurations {
        write_descriptor(w, "AudioChannelConfiguration", d)?;
    }
    for d in &a.content_protections {
        write_descriptor(w, "ContentProtection", d)?;
    }
    for d in &a.essential_properties {
        write_descriptor(w, "EssentialProperty", d)?;
    }
    for d in &a.supplemental_properties {
        write_descriptor(w, "SupplementalProperty", d)?;
    }
    for d in &a.inband_event_streams {
        write_descriptor(w, "InbandEventStream", d)?;
    }
    for l in &a.labels {
        w.write_event(Event::Start(BytesStart::new("Label")))
            .map_err(io_err)?;
        w.write_event(Event::Text(BytesText::new(l)))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("Label")))
            .map_err(io_err)?;
    }
    for p in &a.producer_reference_times {
        write_producer_reference_time(w, p)?;
    }
    for r in &a.resyncs {
        write_resync(w, r)?;
    }
    for d in &a.accessibilities {
        write_descriptor(w, "Accessibility", d)?;
    }
    for d in &a.roles {
        write_descriptor(w, "Role", d)?;
    }
    for b in &a.base_urls {
        write_base_url(w, b)?;
    }
    write_segment_layer(w, &a.segment)?;
    for r in &a.representations {
        write_representation(w, r)?;
    }

    w.write_event(Event::End(BytesEnd::new("AdaptationSet")))
        .map_err(io_err)?;
    Ok(())
}

fn write_representation(w: &mut Writer<Vec<u8>>, r: &Representation) -> Result<(), MpdError> {
    let mut e = BytesStart::new("Representation");
    e.push_attribute(("id", r.id.as_str()));
    if let Some(v) = &r.common.mime_type {
        e.push_attribute(("mimeType", v.as_str()));
    }
    if let Some(v) = &r.common.codecs {
        e.push_attribute(("codecs", v.as_str()));
    }
    e.push_attribute(("bandwidth", r.bandwidth.to_string().as_str()));
    if let Some(v) = r.common.width {
        e.push_attribute(("width", v.to_string().as_str()));
    }
    if let Some(v) = r.common.height {
        e.push_attribute(("height", v.to_string().as_str()));
    }
    if let Some(v) = &r.common.frame_rate {
        e.push_attribute(("frameRate", v.as_str()));
    }
    if let Some(v) = &r.common.sar {
        e.push_attribute(("sar", v.as_str()));
    }
    if let Some(v) = &r.common.audio_sampling_rate {
        e.push_attribute(("audioSamplingRate", v.as_str()));
    }
    if let Some(v) = r.quality_ranking {
        e.push_attribute(("qualityRanking", v.to_string().as_str()));
    }
    if !r.dependency_ids.is_empty() {
        e.push_attribute(("dependencyId", r.dependency_ids.join(" ").as_str()));
    }
    if let Some(v) = &r.common.profiles {
        e.push_attribute(("profiles", v.as_str()));
    }
    if let Some(v) = r.common.max_playout_rate {
        e.push_attribute(("maxPlayoutRate", v.to_string().as_str()));
    }
    if let Some(v) = r.common.coding_dependency {
        e.push_attribute(("codingDependency", if v { "true" } else { "false" }));
    }

    w.write_event(Event::Start(e)).map_err(io_err)?;
    for d in &r.audio_channel_configurations {
        write_descriptor(w, "AudioChannelConfiguration", d)?;
    }
    for d in &r.content_protections {
        write_descriptor(w, "ContentProtection", d)?;
    }
    for d in &r.essential_properties {
        write_descriptor(w, "EssentialProperty", d)?;
    }
    for d in &r.supplemental_properties {
        write_descriptor(w, "SupplementalProperty", d)?;
    }
    for d in &r.inband_event_streams {
        write_descriptor(w, "InbandEventStream", d)?;
    }
    for p in &r.producer_reference_times {
        write_producer_reference_time(w, p)?;
    }
    for rs in &r.resyncs {
        write_resync(w, rs)?;
    }
    for b in &r.base_urls {
        write_base_url(w, b)?;
    }
    write_segment_layer(w, &r.segment)?;

    w.write_event(Event::End(BytesEnd::new("Representation")))
        .map_err(io_err)?;
    Ok(())
}
