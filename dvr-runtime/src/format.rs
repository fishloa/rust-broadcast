//! The archive's vendor tags and its programme `EXT-X-DATERANGE`
//! (RFC 8216bis §4.1, §4.2, §4.4.5.1), built and read only through
//! `broadcast-hls`.
//!
//! RFC 8216bis defines no registry or namespace for vendor tags. §4.1 makes
//! every line beginning `#EXT` a tag and §6.3.1 requires clients to ignore
//! tags they do not recognise, so a standard player skips both tags below.
//! Following the reverse-DNS guidance §4.4.5.1 gives for `X-` attributes, the
//! names carry a full vendor token (`RUSTBROADCAST`) rather than a short
//! prefix that could collide with another vendor or a future standard tag.
//! Their values use the §4.2 attribute-list syntax.

use broadcast_common::hex::hex_encode_upper;
use broadcast_hls::{
    AttrValue, ClientAttrName, CustomTag, CustomTagName, DateRange, DecimalSeconds, ProgramDateTime,
};
use dvb_si::epg::PresentEvent;
use jiff::Timestamp;

/// The playlist-level marker tag's name, without its leading `#`.
pub const ARCHIVE_TAG_NAME: &str = "EXT-X-RUSTBROADCAST-ARCHIVE";

/// The per-segment timing tag's name, without its leading `#`.
pub const TIMING_TAG_NAME: &str = "EXT-X-RUSTBROADCAST-TIMING";

/// The on-disk archive format this crate writes and reads.
pub const ARCHIVE_FORMAT_VERSION: u64 = 1;

/// The `CLASS` of a programme `EXT-X-DATERANGE`.
pub const PROGRAMME_CLASS: &str = "io.github.fishloa.rust-broadcast.programme";

/// The timescale the timing tag's `DTS`/`DUR` are expressed in for format
/// version 1: the Trunk timeline is nanoseconds, so this is exact.
pub const TIMING_TIMESCALE: u64 = 1_000_000_000;

const VERSION_ATTR: &str = "VERSION";
const CONTAINER_ATTR: &str = "CONTAINER";
const PERIOD_ATTR: &str = "PERIOD";
const SEQ_ATTR: &str = "SEQ";
const DTS_ATTR: &str = "DTS";
const DUR_ATTR: &str = "DUR";
const TIMESCALE_ATTR: &str = "TIMESCALE";

/// The decoded marker tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Marker {
    pub(crate) version: u64,
    pub(crate) container: crate::ArchiveContainer,
    pub(crate) period: u32,
}

/// The decoded timing tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    pub(crate) seq: u64,
    pub(crate) dts: u64,
    pub(crate) dur: u64,
    pub(crate) timescale: u64,
}

/// The programme fields a `EXT-X-DATERANGE` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgrammeAttrs {
    pub(crate) service_id: u16,
    pub(crate) event_id: u16,
    pub(crate) title: Option<String>,
    pub(crate) scheduled_start: Option<Timestamp>,
    pub(crate) planned: Option<std::time::Duration>,
}

fn tag_name(name: &'static str) -> Result<CustomTagName, broadcast_hls::Error> {
    CustomTagName::new(name)
}

fn bare(value: String) -> Result<AttrValue, broadcast_hls::Error> {
    AttrValue::bare(value)
}

fn decimal(value: u64) -> String {
    value.to_string()
}

/// Render the playlist-level archive marker line.
pub(crate) fn render_marker(
    container: crate::ArchiveContainer,
    period: crate::PeriodNum,
) -> Result<String, broadcast_hls::Error> {
    let mut tag = CustomTag::new(tag_name(ARCHIVE_TAG_NAME)?);
    tag.set_attr(VERSION_ATTR, bare(decimal(ARCHIVE_FORMAT_VERSION))?)?;
    tag.set_attr(CONTAINER_ATTR, bare(container.name().to_owned())?)?;
    tag.set_attr(PERIOD_ATTR, bare(decimal(u64::from(period.value())))?)?;
    tag.to_tag_line()
}

fn attr<'a>(tag: &'a CustomTag, name: &str) -> Option<&'a AttrValue> {
    tag.attrs().iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn attr_u64(tag: &CustomTag, name: &str) -> Option<u64> {
    attr(tag, name)?.as_str().parse().ok()
}

/// Parse an archive marker line, or `None` when it is not one or is missing a
/// required attribute.
pub(crate) fn parse_marker(line: &str) -> Option<Marker> {
    let tag = CustomTag::parse_tag_line(line).ok()?;
    if tag.name.as_str() != ARCHIVE_TAG_NAME {
        return None;
    }
    let container = match attr(&tag, CONTAINER_ATTR)?.as_str() {
        "FMP4" => crate::ArchiveContainer::Fmp4,
        "TS" => crate::ArchiveContainer::Ts,
        _ => return None,
    };
    Some(Marker {
        version: attr_u64(&tag, VERSION_ATTR)?,
        container,
        period: u32::try_from(attr_u64(&tag, PERIOD_ATTR)?).ok()?,
    })
}

/// Render one segment's timing tag line.
pub(crate) fn render_timing(timing: Timing) -> Result<String, broadcast_hls::Error> {
    let mut tag = CustomTag::new(tag_name(TIMING_TAG_NAME)?);
    tag.set_attr(SEQ_ATTR, bare(decimal(timing.seq))?)?;
    tag.set_attr(DTS_ATTR, bare(decimal(timing.dts))?)?;
    tag.set_attr(DUR_ATTR, bare(decimal(timing.dur))?)?;
    tag.set_attr(TIMESCALE_ATTR, bare(decimal(timing.timescale))?)?;
    tag.to_tag_line()
}

/// Parse a timing tag line, or `None` when it is not one or lacks an
/// attribute.
pub(crate) fn parse_timing(line: &str) -> Option<Timing> {
    let tag = CustomTag::parse_tag_line(line).ok()?;
    if tag.name.as_str() != TIMING_TAG_NAME {
        return None;
    }
    Some(Timing {
        seq: attr_u64(&tag, SEQ_ATTR)?,
        dts: attr_u64(&tag, DTS_ATTR)?,
        dur: attr_u64(&tag, DUR_ATTR)?,
        timescale: attr_u64(&tag, TIMESCALE_ATTR)?,
    })
}

/// A quoted-string-safe title: no `"`, and no control character (RFC 8216bis
/// §4.1).
fn title_is_quotable(title: &str) -> bool {
    !title.chars().any(|c| c == '"' || c.is_control())
}

/// Build the `EXT-X-DATERANGE` identifying `programme`, whose observed start
/// is `start` (the wall time of the segment the record is attached to).
///
/// `END-ON-NEXT=YES` makes a programme end where the next one starts, so no
/// end date is known or written. The title travels as `X-TITLE` when it can
/// be a quoted-string and otherwise as the lossless hex
/// `X-TITLE-UTF8=0x<hex>`.
///
/// # Errors
///
/// Propagates `broadcast-hls`'s attribute-value rejection (a value that
/// cannot be represented in its requested wire form).
pub fn programme_date_range(
    programme: &PresentEvent,
    start: ProgramDateTime,
) -> Result<DateRange, broadcast_hls::Error> {
    let event = &programme.event;
    let id = format!("programme-{}-{}", programme.service_id, event.event_id);
    let mut range = DateRange::new(id)?
        .with_class(PROGRAMME_CLASS)?
        .with_start(start);
    if let Some(duration) = event.duration {
        range = range.with_planned_duration(DecimalSeconds::from_duration(duration));
    }
    range = range.with_end_on_next(true);
    range.set_client_attr(
        ClientAttrName::new("X-DVB-SERVICE-ID")?,
        bare(decimal(u64::from(programme.service_id)))?,
    )?;
    range.set_client_attr(
        ClientAttrName::new("X-DVB-EVENT-ID")?,
        bare(decimal(u64::from(event.event_id)))?,
    )?;
    if let Some(scheduled) = event.start_time {
        let value = scheduled.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
        range.set_client_attr(
            ClientAttrName::new("X-SCHEDULED-START")?,
            AttrValue::quoted(value)?,
        )?;
    }
    if let Some(title) = &event.event_name {
        if title_is_quotable(title) {
            range.set_client_attr(
                ClientAttrName::new("X-TITLE")?,
                AttrValue::quoted(title.clone())?,
            )?;
        } else {
            let hex = format!("0x{}", hex_encode_upper(title.as_bytes()));
            range.set_client_attr(ClientAttrName::new("X-TITLE-UTF8")?, bare(hex)?)?;
        }
    }
    Ok(range)
}

fn client_value(range: &DateRange, name: &str) -> Option<AttrValue> {
    range
        .client_attrs()
        .into_iter()
        .find(|(n, _)| n.as_str() == name)
        .map(|(_, v)| v)
}

fn decode_hex_title(value: AttrValue) -> Option<String> {
    if value.is_quoted() {
        return None;
    }
    let hex = value.as_str().strip_prefix("0x")?;
    let bytes = broadcast_common::hex::hex_decode(hex)?;
    String::from_utf8(bytes).ok()
}

fn decode_scheduled(value: AttrValue) -> Option<Timestamp> {
    value.as_str().parse::<Timestamp>().ok()
}

/// Decode a programme `EXT-X-DATERANGE` line, or `None` when the line is not
/// a DATERANGE or carries no `X-DVB-EVENT-ID`/`X-DVB-SERVICE-ID` pair.
pub(crate) fn parse_programme(line: &str) -> Option<ProgrammeAttrs> {
    let range = DateRange::parse_tag_line(line).ok()?;
    let event_id = u16::try_from(
        client_value(&range, "X-DVB-EVENT-ID")?
            .as_str()
            .parse::<u64>()
            .ok()?,
    )
    .ok()?;
    let service_id = u16::try_from(
        client_value(&range, "X-DVB-SERVICE-ID")?
            .as_str()
            .parse::<u64>()
            .ok()?,
    )
    .ok()?;
    let title = client_value(&range, "X-TITLE")
        .filter(|v| v.is_quoted())
        .map(|v| v.as_str().to_owned())
        .or_else(|| client_value(&range, "X-TITLE-UTF8").and_then(decode_hex_title));
    let scheduled_start = client_value(&range, "X-SCHEDULED-START").and_then(decode_scheduled);
    let planned = range
        .planned_duration()
        .and_then(|d| std::time::Duration::try_from_secs_f64(d.get()).ok());
    Some(ProgrammeAttrs {
        service_id,
        event_id,
        title,
        scheduled_start,
        planned,
    })
}
