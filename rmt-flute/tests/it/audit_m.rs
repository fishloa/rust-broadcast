use rmt_flute::ExtTime;

#[test]
fn ext_time_serialize_output_is_always_parseable() {
    let t = ExtTime {
        sct_low: Some(1),
        ..Default::default()
    };
    ExtTime::parse(&t.to_content()).expect("serializer emitted SCT-Low without SCT-High");
}
