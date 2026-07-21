//! Human-readable decoding of CI link frames, for diagnosing live-CAM
//! exchanges (e.g. issue #337).
//!
//! [`decode_frame`] turns one raw link frame into a one-line annotation —
//! TPDU tag → SPDU tag → APDU tag — so a [`RecordingCaDevice`] capture reads
//! like the byte traces in bug reports without hand-decoding. [`decode_log`]
//! formats a whole captured exchange.
//!
//! [`RecordingCaDevice`]: crate::device::RecordingCaDevice

use crate::device::LinkEvent;
use dvb_ci::resource::ResourceId;
use dvb_ci::spdu::tags as spdu_tags;
use dvb_ci::tag::ApduTag;
use dvb_ci::tpdu::{SbValue, tags as tpdu_tags};

/// Name of a TPDU tag (§A.4), or `"tpdu(0xXX)"` for an unknown one.
fn tpdu_name(tag: u8) -> &'static str {
    match tag {
        tpdu_tags::SB => "T_SB",
        tpdu_tags::RCV => "T_RCV",
        tpdu_tags::CREATE_T_C => "Create_T_C",
        tpdu_tags::C_T_C_REPLY => "C_T_C_Reply",
        tpdu_tags::T_C_ERROR => "T_C_Error",
        tpdu_tags::DATA_LAST => "T_Data_Last",
        tpdu_tags::DATA_MORE => "T_Data_More",
        _ => "T_?",
    }
}

/// Name of a session SPDU tag (§7), or `"spdu(0xXX)"` for an unknown one.
fn spdu_name(tag: u8) -> &'static str {
    match tag {
        spdu_tags::SESSION_NUMBER => "session_number",
        spdu_tags::OPEN_SESSION_REQUEST => "open_session_request",
        spdu_tags::OPEN_SESSION_RESPONSE => "open_session_response",
        spdu_tags::CREATE_SESSION => "create_session",
        spdu_tags::CREATE_SESSION_RESPONSE => "create_session_response",
        spdu_tags::CLOSE_SESSION_REQUEST => "close_session_request",
        spdu_tags::CLOSE_SESSION_RESPONSE => "close_session_response",
        _ => "spdu(?)",
    }
}

/// Decode the APDU at the start of `apdu` (3-byte tag) into `name (9F80xx)`.
fn apdu_label(apdu: &[u8]) -> String {
    match apdu.first_chunk::<3>() {
        Some(&[a, b, c]) => {
            let tag = ApduTag::from_bytes(a, b, c);
            format!("{} ({:02X}{:02X}{:02X})", tag.name(), a, b, c)
        }
        None => "apdu(short)".to_string(),
    }
}

/// Decode the SPDU payload of a data TPDU into a label, recursing into the APDU
/// when the SPDU is a `session_number` wrapper.
fn spdu_label(spdu: &[u8]) -> String {
    match spdu.first().copied() {
        Some(spdu_tags::SESSION_NUMBER) if spdu.len() >= 4 => {
            let nb = u16::from_be_bytes([spdu[2], spdu[3]]);
            let rest = &spdu[4..];
            if rest.is_empty() {
                format!("session {nb}")
            } else {
                format!("session {nb} · {}", apdu_label(rest))
            }
        }
        // Session-management SPDUs: surface the resource_id (+ status/nb) so a
        // live trace shows *which* resource each create/open targets.
        Some(spdu_tags::OPEN_SESSION_REQUEST) if spdu.len() >= 6 => {
            let r = ResourceId(u32::from_be_bytes([spdu[2], spdu[3], spdu[4], spdu[5]]));
            format!("open_session_request {}", r.name())
        }
        Some(spdu_tags::CREATE_SESSION) if spdu.len() >= 8 => {
            let r = ResourceId(u32::from_be_bytes([spdu[2], spdu[3], spdu[4], spdu[5]]));
            let nb = u16::from_be_bytes([spdu[6], spdu[7]]);
            format!("create_session {} (nb {nb})", r.name())
        }
        Some(t @ (spdu_tags::CREATE_SESSION_RESPONSE | spdu_tags::OPEN_SESSION_RESPONSE))
            if spdu.len() >= 9 =>
        {
            let status = spdu[2];
            let r = ResourceId(u32::from_be_bytes([spdu[3], spdu[4], spdu[5], spdu[6]]));
            let nb = u16::from_be_bytes([spdu[7], spdu[8]]);
            format!("{} {} status={status:#04x} nb={nb}", spdu_name(t), r.name())
        }
        Some(t) => spdu_name(t).to_string(),
        None => "empty".to_string(),
    }
}

/// Decode one raw link frame into a one-line annotation.
///
/// Handles the leading TPDU, the SPDU it carries (for `T_Data_*`), and the APDU
/// inside a `session_number` SPDU. Appended `T_SB` data-available bits are noted.
#[must_use]
pub fn decode_frame(frame: &[u8]) -> String {
    let Some(&tag) = frame.first() else {
        return "empty frame".to_string();
    };
    match tag {
        tpdu_tags::SB => match frame.get(3) {
            Some(&sb) => format!(
                "T_SB tcid={} DA={}",
                frame.get(2).copied().unwrap_or(0),
                u8::from(SbValue(sb).data_available())
            ),
            None => "T_SB (short)".to_string(),
        },
        tpdu_tags::CREATE_T_C | tpdu_tags::C_T_C_REPLY | tpdu_tags::RCV | tpdu_tags::T_C_ERROR => {
            format!(
                "{} tcid={}",
                tpdu_name(tag),
                frame.get(2).copied().unwrap_or(0)
            )
        }
        tpdu_tags::DATA_LAST | tpdu_tags::DATA_MORE => {
            // tag · length_field · t_c_id · data(=SPDU) · [appended T_SB].
            // length_field uses the Table-1 codec (long-form for >127 bytes), so
            // decode it rather than assuming a single byte.
            let Ok((len, hdr)) = dvb_ci::length::decode(&frame[1..]) else {
                return format!("{} (bad length_field)", tpdu_name(tag));
            };
            let start = 1 + hdr; // t_c_id offset
            let tcid = frame.get(start).copied().unwrap_or(0);
            // `length` counts t_c_id + data; the SPDU is the data after t_c_id.
            let data_end = (start + len).min(frame.len());
            let spdu = frame.get(start + 1..data_end).unwrap_or(&[]);
            if spdu.is_empty() {
                format!("{} tcid={} (poll)", tpdu_name(tag), tcid)
            } else {
                format!("{} tcid={} · {}", tpdu_name(tag), tcid, spdu_label(spdu))
            }
        }
        _ => format!("{} {:02X?}", tpdu_name(tag), &frame[..frame.len().min(8)]),
    }
}

/// Format a whole captured exchange as a multi-line annotated trace.
#[must_use]
pub fn decode_log(log: &[LinkEvent]) -> String {
    let mut out = String::new();
    for ev in log {
        let line = match ev {
            LinkEvent::Tx(f) => format!("W {}", decode_frame(f)),
            LinkEvent::Rx(f) => format!("R {}", decode_frame(f)),
            LinkEvent::Reset => "  reset()".to_string(),
            LinkEvent::SlotInfo(si) => format!(
                "  slot_info() -> present={} ready={}",
                si.module_present, si.module_ready
            ),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_337_handshake_frames() {
        // Bytes lifted from issue #337's trace.
        assert_eq!(decode_frame(&[0x82, 0x01, 0x01]), "Create_T_C tcid=1");
        assert_eq!(decode_frame(&[0x81, 0x01, 0x01]), "T_RCV tcid=1");
        assert_eq!(decode_frame(&[0x80, 0x02, 0x01, 0x00]), "T_SB tcid=1 DA=0");

        // open_session_request (module): a0 07 01 | 91 04 00 01 00 41 | SB
        let osr = [
            0xA0, 0x07, 0x01, 0x91, 0x04, 0x00, 0x01, 0x00, 0x41, 0x80, 0x02, 0x01, 0x00,
        ];
        assert_eq!(
            decode_frame(&osr),
            "T_Data_Last tcid=1 · open_session_request resource_manager"
        );

        // profile_enq (host): a0 09 01 | 90 02 00 01 | 9f 80 10 00
        let enq = [
            0xA0, 0x09, 0x01, 0x90, 0x02, 0x00, 0x01, 0x9F, 0x80, 0x10, 0x00,
        ];
        assert_eq!(
            decode_frame(&enq),
            "T_Data_Last tcid=1 · session 1 · profile_enq (9F8010)"
        );
    }

    #[test]
    fn decodes_long_form_length_profile_reply() {
        // The module's profile_reply, captured raw on hardware (#337): the
        // length_field is long-form `82 00 09`. A short-form decoder mis-reads
        // tcid as 0x00 and garbles the SPDU.
        let f = [
            0xA0, 0x82, 0x00, 0x09, 0x01, 0x90, 0x02, 0x00, 0x01, 0x9F, 0x80, 0x11, 0x00, 0x80,
            0x02, 0x01, 0x00,
        ];
        assert_eq!(
            decode_frame(&f),
            "T_Data_Last tcid=1 · session 1 · profile (9F8011)"
        );
    }

    #[test]
    fn decodes_log_directions() {
        let log = [
            LinkEvent::Reset,
            LinkEvent::Tx(vec![0x82, 0x01, 0x01]),
            LinkEvent::Rx(vec![0x80, 0x02, 0x01, 0x00]),
        ];
        let s = decode_log(&log);
        assert!(s.contains("  reset()"));
        assert!(s.contains("W Create_T_C tcid=1"));
        assert!(s.contains("R T_SB tcid=1 DA=0"));
    }
}
