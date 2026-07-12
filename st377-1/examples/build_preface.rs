//! Build a `Preface` Header Metadata Set from typed fields, serialize it,
//! then parse it back — SMPTE ST 377-1:2019 Annex A.2.
//!
//! Run with `cargo run -p st377-1 --example build_preface`.

use broadcast_common::{Parse, Serialize};
use st377_1::{InterchangeObjectFields, MxfTimestamp, Preface, VERSION_1_3};

fn main() {
    let preface = Preface {
        interchange: InterchangeObjectFields {
            instance_uid: [0x01; 16],
            generation_uid: None,
            object_class: None,
        },
        last_modified_date: MxfTimestamp {
            year: 2026,
            month: 7,
            day: 12,
            hour: 10,
            minute: 0,
            second: 0,
            msec_div4: 0,
        },
        version: VERSION_1_3,
        object_model_version: Some(1),
        primary_package: None,
        identifications: vec![[0x02; 16]],
        content_storage: [0x03; 16],
        operational_pattern: [0x04; 16],
        essence_containers: vec![[0x05; 16]],
        dm_schemes: Vec::new(),
        dark: Vec::new(),
    };

    let bytes = preface.to_bytes();
    println!("serialized Preface: {} bytes", bytes.len());

    let parsed = Preface::parse(&bytes).expect("parse Preface");
    assert_eq!(parsed, preface);

    println!(
        "last modified: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        parsed.last_modified_date.year,
        parsed.last_modified_date.month,
        parsed.last_modified_date.day,
        parsed.last_modified_date.hour,
        parsed.last_modified_date.minute,
        parsed.last_modified_date.second,
    );
    println!("version: 0x{:04X}", parsed.version);
    println!(
        "{} Identification(s), {} EssenceContainer UL(s)",
        parsed.identifications.len(),
        parsed.essence_containers.len()
    );
}
