//! Wire values and spec labels of the L1 / FEF / timestamp field enums.
//!
//! Names are the EN 302 755 / ETSI TS 102 773 Table tokens; the loop also pins
//! `from_u8` -> `to_u8` for every byte, so a constant `to_u8` cannot pass.

use dvb_t2mi::packet::PacketType;
use dvb_t2mi::payload::fef_null::{S1Field, S2Field1};
use dvb_t2mi::payload::{
    AuxStreamType, Bandwidth, FrequencySource, GuardInterval, L1CodeRate, L1FecType, L1Modulation,
    PaprReductionV0, PaprReductionVn, PilotPattern, PlpCodeRate, PlpFecType, PlpMode,
    PlpModulation, PlpPayloadType, PlpType, PrbsType, SubpartVariety, T2Version, TxInputStreamType,
};

fn assert_names<T, F, G, N>(label: &str, mask: u8, names: &[&str], from: F, to: G, name: N)
where
    T: Copy + core::fmt::Display,
    F: Fn(u8) -> T,
    G: Fn(T) -> u8,
    N: Fn(T) -> &'static str,
{
    for v in 0u8..=255 {
        let value = from(v);
        assert_eq!(to(value), v & mask, "{label}: wire round-trip for {v:#04x}");
    }
    for (index, expected) in names.iter().enumerate() {
        let value = from(index as u8);
        assert_eq!(name(value), *expected, "{label}: name of {index}");
        assert_eq!(value.to_string(), *expected, "{label}: display of {index}");
    }
    if (names.len() as u8) <= mask {
        assert_eq!(
            name(from(names.len() as u8)),
            "reserved",
            "{label}: unallocated value"
        );
    }
}

#[test]
fn tx_input_stream_type_values_and_names() {
    assert_names(
        "TxInputStreamType",
        0xFF,
        &["TS", "Generic Stream", "TS + Generic"],
        TxInputStreamType::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn guard_interval_values_and_names() {
    assert_names(
        "GuardInterval",
        0x07,
        &["1/32", "1/16", "1/8", "1/4", "1/128", "19/128", "19/256"],
        GuardInterval::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn l1_modulation_values_and_names() {
    assert_names(
        "L1Modulation",
        0x0F,
        &["BPSK", "QPSK", "16-QAM", "64-QAM"],
        L1Modulation::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn l1_code_rate_values_and_names() {
    assert_names(
        "L1CodeRate",
        0x03,
        &["1/2"],
        L1CodeRate::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn l1_fec_type_values_and_names() {
    assert_names(
        "L1FecType",
        0x03,
        &["16K LDPC"],
        L1FecType::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn pilot_pattern_values_and_names() {
    assert_names(
        "PilotPattern",
        0x0F,
        &["PP1", "PP2", "PP3", "PP4", "PP5", "PP6", "PP7", "PP8"],
        PilotPattern::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn t2_version_values_and_names() {
    assert_names(
        "T2Version",
        0x0F,
        &["1.1.1", "1.2.1", "1.3.1"],
        T2Version::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn plp_type_values_and_names() {
    assert_names(
        "PlpType",
        0x07,
        &["Common", "Data Type 1", "Data Type 2"],
        PlpType::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn plp_payload_type_values_and_names() {
    assert_names(
        "PlpPayloadType",
        0x1F,
        &["GFPS", "GCS", "GSE", "TS"],
        PlpPayloadType::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn plp_modulation_values_and_names() {
    assert_names(
        "PlpModulation",
        0x07,
        &["QPSK", "16-QAM", "64-QAM", "256-QAM"],
        PlpModulation::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn plp_fec_type_values_and_names() {
    assert_names(
        "PlpFecType",
        0x03,
        &["16K LDPC", "64K LDPC"],
        PlpFecType::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn plp_mode_values_and_names() {
    assert_names(
        "PlpMode",
        0x03,
        &["Not specified", "Normal", "High Efficiency"],
        PlpMode::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn aux_stream_type_values_and_names() {
    assert_names(
        "AuxStreamType",
        0x0F,
        &["TX-SIG"],
        AuxStreamType::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn papr_reduction_v0_values_and_names() {
    assert_names(
        "PaprReductionV0",
        0x0F,
        &[
            "No PAPR reduction",
            "ACE-PAPR only",
            "TR-PAPR only",
            "ACE and TR",
        ],
        PaprReductionV0::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn papr_reduction_vn_values_and_names() {
    assert_names(
        "PaprReductionVn",
        0x0F,
        &[
            "L1-ACE, TR on P2 only",
            "L1-ACE and ACE",
            "L1-ACE and TR",
            "L1-ACE, ACE and TR",
        ],
        PaprReductionVn::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn plp_code_rate_values_and_names() {
    assert_names(
        "PlpCodeRate",
        0x07,
        &["1/2", "3/5", "2/3", "3/4", "4/5", "5/6"],
        PlpCodeRate::from_u8,
        |v| v.to_u8(),
        |v| v.name(),
    );
}

#[test]
fn s2_field1_fft_size_labels() {
    assert_names(
        "S2Field1",
        0x07,
        &["1k", "2k", "4k", "8k", "16k", "32k", "reserved", "reserved"],
        S2Field1::from_u8,
        |v| v.to_u8(),
        |v| v.fft_size(),
    );
    assert_eq!(S2Field1::Fft8k.name(), "8k");
}

#[test]
fn s1_field_labels() {
    let names = [
        "T2_SISO",
        "T2_MISO",
        "Non-T2",
        "T2_LITE_SISO",
        "T2_LITE_MISO",
        "reserved",
        "reserved",
        "reserved",
    ];
    for (value, expected) in names.iter().enumerate() {
        let field = S1Field::try_from(value as u8).unwrap();
        assert_eq!(field.name(), *expected);
        assert_eq!(u8::from(field), value as u8);
    }
}

#[test]
fn subpart_variety_and_prbs_type_labels() {
    let names = ["Null", "IQ", "PRBS", "TX-SIG FEF"];
    for (value, expected) in names.iter().enumerate() {
        let variety = SubpartVariety::try_from(value as u16).unwrap();
        assert_eq!(variety.name(), *expected);
        assert_eq!(u16::from(variety), value as u16);
    }
    assert_eq!(PrbsType::UserDefined.name(), "user-defined");
    assert_eq!(u8::from(PrbsType::UserDefined), 0);
}

#[test]
fn frequency_source_labels() {
    let names = [
        "L1-current data",
        "individual addressing",
        "manual per modulator",
    ];
    for (value, expected) in names.iter().enumerate() {
        let source = FrequencySource::try_from(value as u8).unwrap();
        assert_eq!(source.name(), *expected);
        assert_eq!(u8::from(source), value as u8);
    }
}

#[test]
fn bandwidth_labels() {
    let names = ["1.7 MHz", "5 MHz", "6 MHz", "7 MHz", "8 MHz", "10 MHz"];
    for (value, expected) in names.iter().enumerate() {
        let bandwidth = Bandwidth::try_from(value as u8).unwrap();
        assert_eq!(bandwidth.name(), *expected);
        assert_eq!(u8::from(bandwidth), value as u8);
    }
}

#[test]
fn packet_type_labels() {
    let names = [
        (0x00u8, "BBFRAME"),
        (0x01, "Auxiliary stream I/Q data"),
        (0x02, "Arbitrary cell insertion"),
        (0x10, "L1-current"),
        (0x11, "L1-future"),
        (0x12, "P2 bias balancing cells"),
        (0x20, "Timestamp"),
        (0x21, "Individual addressing"),
        (0x30, "FEF part: Null"),
        (0x31, "FEF part: I/Q data"),
        (0x32, "FEF part: composite"),
        (0x33, "FEF sub-part"),
    ];
    for (value, expected) in names {
        let packet_type = PacketType::try_from(value).unwrap();
        assert_eq!(packet_type.name(), expected);
        assert_eq!(packet_type.to_string(), expected);
        assert_eq!(u8::from(packet_type), value);
    }
}
