#![no_main]

use dvb_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Try to parse the DSM-CC section first.
    let section = match dvb_si::tables::dsmcc::DsmccSection::parse(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    let payload = section.payload;

    // Try parsing payload as an UnMessage (DSI or DII).
    if let Ok(un) = dvb_si::carousel::messages::UnMessage::parse(payload) {
        let mut reassembler = dvb_si::carousel::ModuleReassembler::new();
        if let dvb_si::carousel::messages::UnMessage::Dii(dii) = un {
            reassembler.note_dii(&dii);
        }
    }

    // Try parsing payload as a DownloadDataBlock.
    if let Ok(ddb) = dvb_si::carousel::messages::DownloadDataBlock::parse(payload) {
        let mut reassembler = dvb_si::carousel::ModuleReassembler::new();
        let _ = reassembler.feed_ddb(&ddb);
    }
});
