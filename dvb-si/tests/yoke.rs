//! `yoke` feature integration tests: an owned, parsed view outlives the source
//! buffer, crosses a thread boundary, and clones cheaply — all without a
//! re-parse. (Issue #27.)
#![cfg(feature = "yoke")]

use std::sync::Arc;

use dvb_common::{Parse, Serialize};
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::owned::Owned;
use dvb_si::tables::sdt::{Sdt, SdtKind, SdtService};

/// Build a minimal two-service SDT section through the serializer.
fn sdt_section() -> Vec<u8> {
    let sdt = Sdt {
        kind: SdtKind::Actual,
        transport_stream_id: 0x1234,
        version_number: 3,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        original_network_id: 0x5678,
        services: vec![
            SdtService {
                service_id: 0x0A,
                eit_schedule_flag: true,
                eit_present_following_flag: true,
                running_status: 4,
                free_ca_mode: false,
                descriptors: DescriptorLoop::new(&[]),
            },
            SdtService {
                service_id: 0x0B,
                eit_schedule_flag: false,
                eit_present_following_flag: false,
                running_status: 1,
                free_ca_mode: true,
                descriptors: DescriptorLoop::new(&[]),
            },
        ],
    };
    let mut buf = vec![0u8; sdt.serialized_len()];
    sdt.serialize_into(&mut buf).unwrap();
    buf
}

#[test]
fn owned_outlives_source_vec() {
    // The source `Vec<u8>` is consumed into the Arc cart and dropped from this
    // scope; the owned view must keep working afterwards.
    let owned: Owned<Sdt<'static>> = {
        let source: Vec<u8> = sdt_section();
        let cart: Arc<[u8]> = Arc::from(source); // `source` moved/consumed here
        Owned::try_new(cart, |b| Sdt::parse(b)).unwrap()
    };

    let sdt = owned.get();
    assert_eq!(sdt.transport_stream_id, 0x1234);
    assert_eq!(sdt.original_network_id, 0x5678);
    assert_eq!(sdt.services.len(), 2);
    assert_eq!(sdt.services[0].service_id, 0x0A);
    assert_eq!(sdt.services[1].service_id, 0x0B);
}

#[test]
fn owned_crosses_thread_boundary() {
    let cart: Arc<[u8]> = Arc::from(sdt_section());
    let owned: Owned<Sdt<'static>> = Owned::try_new(cart, |b| Sdt::parse(b)).unwrap();

    let handle = std::thread::spawn(move || {
        let sdt = owned.get();
        (sdt.transport_stream_id, sdt.services.len())
    });
    let (tsid, n) = handle.join().unwrap();
    assert_eq!(tsid, 0x1234);
    assert_eq!(n, 2);
}

#[test]
fn clone_is_cheap_and_both_read() {
    let cart: Arc<[u8]> = Arc::from(sdt_section());
    let backing_ptr = cart.as_ptr();
    let owned: Owned<Sdt<'static>> = Owned::try_new(cart, |b| Sdt::parse(b)).unwrap();

    let clone = owned.clone();

    // A clone is a refcount bump: both share the same backing allocation, no
    // re-parse, no second copy of the bytes.
    assert_eq!(owned.backing_bytes().as_ptr(), backing_ptr);
    assert_eq!(clone.backing_bytes().as_ptr(), backing_ptr);

    assert_eq!(owned.get().transport_stream_id, 0x1234);
    assert_eq!(clone.get().transport_stream_id, 0x1234);
    assert_eq!(owned.get().services.len(), clone.get().services.len());
}

#[test]
fn owned_descriptor_loop() {
    // The descriptor loop view yokes just as well as a whole table.
    let raw = [
        0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
    ];
    let cart: Arc<[u8]> = Arc::from(&raw[..]);
    let owned: Owned<DescriptorLoop<'static>> = Owned::new(cart, |b| DescriptorLoop::new(b));

    assert_eq!(owned.get().raw().len(), raw.len());
    assert_eq!(owned.get().iter().count(), 1);
}
