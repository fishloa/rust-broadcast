#![no_main]

use broadcast_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = dvb_ci::tpdu::CommandTpdu::parse(data);
    let _ = dvb_ci::tpdu::ResponseTpdu::parse(data);
    if let Ok(o) = dvb_ci::tpdu::TcObject::parse(data) {
        // A parsed connection object must re-serialize (tag already validated).
        assert!(dvb_ci::tpdu::tc_object_bytes(&o).is_ok());
    }
});
