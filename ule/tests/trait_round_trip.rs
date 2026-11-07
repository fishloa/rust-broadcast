//! `Sndu` and `PayloadChain` through `broadcast_common::{Parse, Serialize}`.

use broadcast_common::{Parse, Serialize};
use ule::{Error, Sndu};

fn fixture() -> Vec<u8> {
    test_fixtures::read("ule/appendix-b.bin")
}

#[test]
fn sndu_round_trips_through_the_traits_on_the_rfc_vector() {
    let data = fixture();
    let sndu = <Sndu as Parse>::parse(&data).unwrap();
    let wire = Serialize::try_to_bytes(&sndu).unwrap();
    assert_eq!(wire, data);
    assert_eq!(<Sndu as Parse>::parse(&wire).unwrap(), sndu);
}

#[test]
fn payload_chain_serializes_through_the_trait() {
    let data = fixture();
    let sndu = <Sndu as Parse>::parse(&data).unwrap();
    let wire = Serialize::try_to_bytes(&sndu.payload).unwrap();
    assert_eq!(wire.len(), Serialize::serialized_len(&sndu.payload));
    assert!(wire.ends_with(sndu.pdu()));
}

#[test]
fn short_output_is_output_buffer_too_small() {
    let data = fixture();
    let sndu = <Sndu as Parse>::parse(&data).unwrap();
    let have = data.len() - 1;
    let mut out = vec![0u8; have];
    assert_eq!(
        Serialize::serialize_into(&sndu, &mut out),
        Err(Error::OutputBufferTooSmall {
            need: data.len(),
            have
        })
    );
}
