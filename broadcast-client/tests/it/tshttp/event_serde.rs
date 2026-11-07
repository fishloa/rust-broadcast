#![cfg(feature = "serde")]

use core::marker::PhantomData;

use broadcast_client::tshttp::{Config, Event, StarveKind, TsHttpClient};
use serde::de::DeserializeOwned;
use transmux::StreamingTsDemux;

struct Probe<T>(PhantomData<T>);

trait Fallback {
    const DESERIALIZES: bool = false;
}
impl<T> Fallback for Probe<T> {}

impl<T: DeserializeOwned> Probe<T> {
    const DESERIALIZES: bool = true;
}

#[test]
fn event_is_serialize_only() {
    let flags = [
        Probe::<Event>::DESERIALIZES,
        Probe::<StarveKind>::DESERIALIZES,
        Probe::<Config>::DESERIALIZES,
    ];
    assert_eq!(flags, [false, true, true]);
}

#[test]
fn demux_event_serializes_as_its_bare_tag() {
    let mut d = StreamingTsDemux::new();
    d.feed(&test_fixtures::read("ts/h264-aac.ts"));
    let ev = std::iter::from_fn(|| d.poll_event())
        .next()
        .expect("demuxer emits an event");
    assert_eq!(
        serde_json::to_string(&Event::Demux(ev)).expect("serialize"),
        "\"Demux\""
    );
}

#[test]
fn plain_events_serialize_with_their_fields() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = c.poll();
    c.on_response(404, &[], 0);
    let e = c.poll_event().expect("event");
    assert_eq!(
        serde_json::to_string(&e).expect("serialize"),
        "{\"NotFound\":{\"status\":404}}"
    );
}
