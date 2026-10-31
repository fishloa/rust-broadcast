use broadcast_hls::{AttrValue, MasterPlaylist, Variant};

#[test]
fn supplemental_codecs_round_trips_as_a_quoted_string() {
    for value in ["dvh1.08.03/db1p", "dvh1.08.07/db4h", "hvc1.2.4.H153.90/cdm4"] {
        let attr = (
            "SUPPLEMENTAL-CODECS".to_string(),
            AttrValue::for_attr("SUPPLEMENTAL-CODECS", value).unwrap(),
        );
        let master = MasterPlaylist {
            variants: vec![Variant {
                bandwidth: 1000,
                codecs: Some("hvc1.2.4.L150.B0".to_string()),
                uri: "v.m3u8".to_string(),
                extra_attrs: vec![attr.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let rendered = master.to_m3u8().unwrap();
        assert!(
            rendered.contains(&format!("SUPPLEMENTAL-CODECS=\"{value}\"")),
            "{rendered}"
        );
        let reparsed = MasterPlaylist::parse(&rendered).unwrap();
        assert_eq!(reparsed.variants[0].extra_attrs, vec![attr]);
        assert_eq!(reparsed.to_m3u8().unwrap(), rendered);
    }
}
