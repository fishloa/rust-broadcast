/// Build an ECMG⇔SCS `channel_setup` message from typed fields, serialize it
/// (recomputing message_length + each parameter_length), and dump the wire
/// bytes.
///
/// ```sh
/// cargo run -p dvb-simulcrypt --example build_channel_setup
/// ```
use dvb_common::traits::{Parse, Serialize};
use dvb_simulcrypt::{
    EcmgScsMessageType, EcmgScsParameterType, Interface, MessageType, Parameter, ParameterType,
    SimulcryptMessage,
};

fn main() {
    // channel_setup (Table, §5.4.1): ECM_channel_id (1) + Super_CAS_id (1).
    let ecm_channel_id = [0x00u8, 0x2A]; // ECM_channel_id = 0x002A
    let super_cas_id = [0x00u8, 0x01, 0x00, 0x02]; // CA_system_id | CA_subsystem_id

    let msg = SimulcryptMessage::new(
        Interface::EcmgScs.protocol_version(),
        MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
        vec![
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
                &ecm_channel_id,
            ),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::SuperCasId),
                &super_cas_id,
            ),
        ],
    );

    let mut bytes = vec![0u8; msg.serialized_len()];
    let n = msg.serialize_into(&mut bytes).unwrap();

    println!("interface: {}", msg.interface());
    println!(
        "message_type: {} (0x{:04X})",
        msg.message_type,
        msg.message_type.to_u16()
    );
    println!("protocol_version: 0x{:02X}", msg.protocol_version);
    println!("message_length (body): {}", msg.body_len());
    println!("parameters: {}", msg.parameters.len());
    for p in &msg.parameters {
        print!("  {} (0x{:04X}) =", p.ptype, p.ptype.to_u16());
        for b in p.value {
            print!(" {b:02X}");
        }
        println!();
    }
    print!("wire bytes ({n}):");
    for b in &bytes {
        print!(" {b:02X}");
    }
    println!();

    // Round-trip: parse against the same interface, expect equality.
    assert_eq!(
        SimulcryptMessage::parse_on(Interface::EcmgScs, &bytes).unwrap(),
        msg
    );
    // The default `Parse` impl also targets ECMG⇔SCS.
    assert_eq!(SimulcryptMessage::parse(&bytes).unwrap(), msg);
    println!("round-trip: OK");
}
