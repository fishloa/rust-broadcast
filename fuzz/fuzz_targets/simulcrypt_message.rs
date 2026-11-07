#![no_main]

use broadcast_common::Serialize;
use dvb_simulcrypt::{Interface, SimulcryptMessage};
use libfuzzer_sys::fuzz_target;

const INTERFACES: [Interface; 3] = [
    Interface::EcmgScs,
    Interface::EmmgPdgMux,
    Interface::CpSigPSig,
];

fuzz_target!(|data: &[u8]| {
    let _ = SimulcryptMessage::frame_len(data);
    for iface in INTERFACES {
        let Ok(parsed) = SimulcryptMessage::parse_on(iface, data) else {
            continue;
        };
        let bytes = parsed.try_to_bytes().expect("a parsed message serializes");
        let reparsed =
            SimulcryptMessage::parse_on(iface, &bytes).expect("serialized message reparses");
        assert_eq!(parsed, reparsed, "simulcrypt parse->serialize->parse");
    }
});
