//! Live-capture validation of the DSM-CC carousel message layouts.
//!
//! `tests/fixtures/m6-single.ts` (French TNT, M6 HbbTV object carousel,
//! PID 0x00AB) carries a DSI, a DII and DDB sections. These tests pin the
//! hand-transcribed layouts in `docs/iso_13818_6_carousel.md` against real
//! broadcast bytes: DSM-CC section framing → typed DSI/DII/DDB messages →
//! cross-message invariants (downloadId linkage, DVB transactionId rules,
//! DII geometry vs DDB block sizes).

use dvb_common::{Parse, Serialize};
use dvb_si::carousel::{DownloadDataBlock, ModuleReassembler, UnMessage};
use dvb_si::tables::dsmcc::DsmccSection;
use dvb_si::ts::{SectionReassembler, TsPacket, TS_PACKET_SIZE};

/// Read a TS file and return all reassembled sections for a given PID.
fn extract_sections_for_pid(path: &str, target_pid: u16) -> Vec<Vec<u8>> {
    let data = std::fs::read(path).expect("read fixture");
    let mut reassembler = SectionReassembler::default();
    let mut sections = Vec::new();

    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        let pkt = match TsPacket::parse(chunk) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pkt.header.pid != target_pid {
            continue;
        }
        if let Some(payload) = pkt.payload {
            reassembler.feed(payload, pkt.header.pusi);
            while let Some(sec) = reassembler.pop_section() {
                sections.push(sec.to_vec());
            }
        }
    }
    sections
}

fn m6_dsmcc_sections() -> Vec<Vec<u8>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    extract_sections_for_pid(path, 0x00AB)
}

#[test]
fn fixture_m6_un_messages_parse_typed() {
    let mut dsi_count = 0;
    let mut dii_count = 0;

    for sec in &m6_dsmcc_sections() {
        if sec.first() != Some(&0x3B) {
            continue;
        }
        let section = DsmccSection::parse(sec).expect("DSM-CC section parse");
        match UnMessage::parse(section.payload).expect("U-N message parse") {
            UnMessage::Dsi(dsi) => {
                // DVB profile (TR 101 202): serverId all 0xFF; §4.7.9: the
                // 2 LSBs of a DSI transactionId are 0x0000.
                assert_eq!(dsi.server_id, [0xFF; 20]);
                assert_eq!(dsi.transaction_id & 0xFFFF, 0x0000);
                dsi_count += 1;
            }
            UnMessage::Dii(dii) => {
                // §4.7.9: DII transactionId 2 LSBs are non-zero.
                assert_ne!(dii.transaction_id & 0xFFFF, 0x0000);
                assert!(dii.block_size > 0, "DII must announce a block size");
                assert!(!dii.modules.is_empty(), "DII should describe modules");
                dii_count += 1;
            }
            _ => {}
        }
    }
    assert!(dsi_count >= 1, "capture should contain a DSI");
    assert!(dii_count >= 1, "capture should contain a DII");
}

/// The capture ends mid-DDB (the 0x3C section is 1907 bytes; the file stops
/// after ~551), so no COMPLETE DDB section can be reassembled from it. The
/// dsmccDownloadDataHeader is fully present in the section prefix though —
/// pull it straight from the first 0x3C PUSI packet and pin the layout +
/// the downloadId linkage to the DII against real broadcast bytes.
#[test]
fn fixture_m6_ddb_header_prefix_links_to_dii() {
    let sections = m6_dsmcc_sections();
    let mut dii_download_ids = Vec::new();
    for sec in &sections {
        if sec.first() != Some(&0x3B) {
            continue;
        }
        let s = DsmccSection::parse(sec).expect("section");
        if let UnMessage::Dii(d) = UnMessage::parse(s.payload).expect("U-N") {
            dii_download_ids.push(d.download_id);
        }
    }
    assert!(!dii_download_ids.is_empty(), "capture should contain a DII");

    // Find the start of the 0x3C section directly in the TS packets.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    let data = std::fs::read(path).expect("read fixture");
    let mut ddb_prefix: Option<Vec<u8>> = None;
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        let pkt = match TsPacket::parse(chunk) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pkt.header.pid != 0x00AB || !pkt.header.pusi {
            continue;
        }
        if let Some(payload) = pkt.payload {
            let ptr = payload[0] as usize;
            let start = 1 + ptr;
            if start < payload.len() && payload[start] == 0x3C {
                ddb_prefix = Some(payload[start..].to_vec());
                break;
            }
        }
    }
    let prefix = ddb_prefix.expect("capture should contain the start of a 0x3C section");

    // 8-byte DSM-CC section header, then the dsmccDownloadDataHeader.
    let msg = &prefix[8..];
    assert_eq!(msg[0], 0x11, "protocolDiscriminator");
    assert_eq!(msg[1], 0x03, "dsmccType = U-N download");
    assert_eq!(
        u16::from_be_bytes([msg[2], msg[3]]),
        0x1003,
        "messageId = DDB"
    );
    let download_id = u32::from_be_bytes([msg[4], msg[5], msg[6], msg[7]]);
    assert!(
        dii_download_ids.contains(&download_id),
        "DDB downloadId {download_id:#010x} must match a DII ({dii_download_ids:?})"
    );
}

#[test]
fn fixture_m6_messages_round_trip() {
    for sec in &m6_dsmcc_sections() {
        let section = match DsmccSection::parse(sec) {
            Ok(s) => s,
            Err(_) => continue,
        };
        match section.table_id {
            0x3B => {
                let msg = UnMessage::parse(section.payload).expect("U-N parse");
                let mut buf = vec![0u8; msg.serialized_len()];
                msg.serialize_into(&mut buf).expect("U-N serialize");
                // Byte-exact round trip against the broadcast bytes.
                assert_eq!(buf.as_slice(), section.payload);
            }
            0x3C => {
                let ddb = DownloadDataBlock::parse(section.payload).expect("DDB parse");
                let mut buf = vec![0u8; ddb.serialized_len()];
                ddb.serialize_into(&mut buf).expect("DDB serialize");
                assert_eq!(buf.as_slice(), section.payload);
            }
            _ => {}
        }
    }
}

/// The DII announced by the live capture feeds the reassembler, which must
/// accept its modules into pending slots (the capture is too short to carry
/// the module bytes themselves — completion is exercised synthetically below).
#[test]
fn fixture_m6_dii_arms_the_reassembler() {
    let mut reasm = ModuleReassembler::new();
    for sec in &m6_dsmcc_sections() {
        if sec.first() != Some(&0x3B) {
            continue;
        }
        let s = DsmccSection::parse(sec).expect("section");
        if let UnMessage::Dii(dii) = UnMessage::parse(s.payload).expect("U-N") {
            reasm.note_dii(&dii);
        }
    }
    assert!(
        reasm.pending() >= 1,
        "the capture's DII should arm at least one module slot"
    );
}

/// End-to-end through every layer with synthetic sections built by our own
/// serializers: DSM-CC section framing → U-N/DDB message parse → module
/// reassembly to completion.
#[test]
fn synthetic_full_pipeline_reassembles_module() {
    use dvb_si::carousel::{Dii, DiiModule};

    let module_bytes: Vec<u8> = (0u8..200).cycle().take(700).collect();
    let block_size = 256usize;

    let dii = UnMessage::Dii(Dii {
        transaction_id: 0x8000_0002,
        adaptation: &[],
        download_id: 0xCAFE,
        block_size: block_size as u16,
        window_size: 0,
        ack_period: 0,
        t_c_download_window: 0,
        t_c_download_scenario: 0,
        compatibility_descriptor: &[],
        modules: vec![DiiModule {
            module_id: 41,
            module_size: module_bytes.len() as u32,
            module_version: 2,
            module_info: &[],
        }],
        private_data: &[],
    });
    let mut dii_buf = vec![0u8; dii.serialized_len()];
    dii.serialize_into(&mut dii_buf).unwrap();

    let mut ddb_bufs = Vec::new();
    for (n, blk) in module_bytes.chunks(block_size).enumerate() {
        let ddb = DownloadDataBlock {
            download_id: 0xCAFE,
            adaptation: &[],
            module_id: 41,
            module_version: 2,
            block_number: n as u16,
            block_data: blk,
        };
        let mut buf = vec![0u8; ddb.serialized_len()];
        ddb.serialize_into(&mut buf).unwrap();
        ddb_bufs.push(buf);
    }

    // Frame each message in a DSM-CC section via the section serializer,
    // then walk the whole stack back down.
    let mut reasm = ModuleReassembler::new();
    let mut completed = None;
    let wrap = |table_id: u8, payload: &[u8]| -> Vec<u8> {
        let sec = DsmccSection {
            table_id,
            extension_id: 0,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            payload,
        };
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        buf
    };

    let dii_section = wrap(0x3B, &dii_buf);
    let parsed = DsmccSection::parse(&dii_section).unwrap();
    if let UnMessage::Dii(d) = UnMessage::parse(parsed.payload).unwrap() {
        reasm.note_dii(&d);
    }
    for raw in &ddb_bufs {
        let section = wrap(0x3C, raw);
        let parsed = DsmccSection::parse(&section).unwrap();
        let ddb = DownloadDataBlock::parse(parsed.payload).unwrap();
        if let Some(m) = reasm.feed_ddb(&ddb) {
            completed = Some(m);
        }
    }

    let module = completed.expect("module should complete");
    assert_eq!(module.key.download_id, 0xCAFE);
    assert_eq!(module.key.module_id, 41);
    assert_eq!(module.data, module_bytes);
    assert_eq!(reasm.pending(), 0);
}
