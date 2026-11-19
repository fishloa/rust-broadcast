//! Spec labels of the BBFrame field enums — the #204 `name()`/`Display` pair.

use dvb_bbframe::header::{Mode, RollOff, TsGs};
use dvb_bbframe::issy::{BufsUnit, Issy, SignallingKind};

fn assert_label<T: core::fmt::Display>(value: T, name: &str, expected: &str) {
    assert_eq!(name, expected);
    assert_eq!(value.to_string(), expected);
}

#[test]
fn ts_gs_labels() {
    assert_label(TsGs::Gfps, TsGs::Gfps.name(), "GFPS");
    assert_label(TsGs::Ts, TsGs::Ts.name(), "TS");
    assert_label(TsGs::Gcs, TsGs::Gcs.name(), "GCS");
    assert_label(TsGs::Gse, TsGs::Gse.name(), "GSE");
}

#[test]
fn mode_labels() {
    assert_label(Mode::Normal, Mode::Normal.name(), "Normal");
    assert_label(
        Mode::HighEfficiency,
        Mode::HighEfficiency.name(),
        "High Efficiency",
    );
}

#[test]
fn roll_off_labels() {
    assert_label(RollOff::Alpha035, RollOff::Alpha035.name(), "α=0.35");
    assert_label(RollOff::Alpha025, RollOff::Alpha025.name(), "α=0.25");
    assert_label(RollOff::Alpha020, RollOff::Alpha020.name(), "α=0.20");
    assert_label(
        RollOff::Reserved,
        RollOff::Reserved.name(),
        "reserved/S2X-low",
    );
}

#[test]
fn bufs_unit_labels() {
    assert_label(BufsUnit::Bits, BufsUnit::Bits.name(), "bits");
    assert_label(BufsUnit::Kbits, BufsUnit::Kbits.name(), "Kbits");
    assert_label(BufsUnit::Mbits, BufsUnit::Mbits.name(), "Mbits");
    assert_label(BufsUnit::Kbits8, BufsUnit::Kbits8.name(), "8 Kbits");
}

#[test]
fn signalling_kind_labels() {
    let bufs = SignallingKind::Bufs {
        bufs: 10,
        units: BufsUnit::Bits,
    };
    assert_label(bufs, bufs.name(), "BUFS");

    let tto = SignallingKind::Tto {
        tto_e: 1,
        tto_m: 2,
        tto_l: 3,
    };
    assert_label(tto, tto.name(), "TTO");

    let bufstat = SignallingKind::BufStat {
        bufstat: 4,
        units: BufsUnit::Kbits,
    };
    assert_label(bufstat, bufstat.name(), "BUFSTAT");

    let reserved = SignallingKind::Reserved(0x123);
    assert_label(reserved, reserved.name(), "reserved");
}

#[test]
fn issy_labels() {
    let short = Issy::IscrShort(0x1234);
    assert_label(short, short.name(), "ISCR short");

    let long = Issy::IscrLong(0x12_3456);
    assert_label(long, long.name(), "ISCR long");

    let signalling = Issy::Signalling(SignallingKind::Tto {
        tto_e: 0,
        tto_m: 0,
        tto_l: 0,
    });
    assert_label(signalling, signalling.name(), "signalling");
}
