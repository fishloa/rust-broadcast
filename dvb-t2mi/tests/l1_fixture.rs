//! Fixture oracle: validate L1-pre / L1-post decoding against real broadcast bytes.
//!
//! Fixture: `tests/fixtures/colombia-capital-t2mi.ts` — the same conformant T2-MI
//! capture used by `real_capture.rs`. We pump it, find the first L1-current payload,
//! and assert ground-truth decoded field values verified from the wire bytes.
#![cfg(feature = "ts")]

use dvb_t2mi::payload::fef_null::S1Field;
use dvb_t2mi::payload::AnyPayload;
use dvb_t2mi::payload::{
    GuardInterval, L1Modulation, PilotPattern, PlpPayloadType, PlpType, T2Version,
    TxInputStreamType,
};
use dvb_t2mi::pump::T2miPump;

const T2MI_PID: u16 = 0x0040;
const TS_PACKET: usize = 188;

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/colombia-capital-t2mi.ts"
    );
    std::fs::read(path).expect("fixture present")
}

#[test]
fn l1_current_fixture_oracle() {
    let data = fixture();
    let mut pump = T2miPump::new(T2MI_PID);

    let mut first_l1 = None;
    'outer: for pkt in data.chunks(TS_PACKET) {
        for event in pump.feed_ts(pkt) {
            if let Ok(AnyPayload::L1Current(payload)) = event.payload() {
                first_l1 = Some(payload.l1_current_data.to_vec());
                break 'outer;
            }
        }
    }

    let l1_data = first_l1.expect("at least one L1Current packet in fixture");

    // ── l1_current_data length ────────────────────────────────────────────────
    assert_eq!(
        l1_data.len(),
        67,
        "l1_current_data must be 67 bytes (21 L1PRE + 2+24 CONF + 2+16 DYN + 2+0 EXT)"
    );

    // Re-parse via the lazy accessor path (borrowing from l1_data)
    let payload = dvb_t2mi::payload::L1CurrentPayload {
        frame_idx: 0,
        freq_source: dvb_t2mi::payload::FrequencySource::UseL1CurrentData,
        l1_current_data: &l1_data,
    };

    // ── L1Pre fields ──────────────────────────────────────────────────────────
    let pre = payload.l1_pre().expect("l1_pre() must succeed");

    assert_eq!(pre.type_, TxInputStreamType::TsOnly, "TYPE");
    assert_eq!(pre.s1, S1Field::V0, "S1 (T2_SISO)");
    assert_eq!(pre.guard_interval, GuardInterval::G1_8, "GUARD_INTERVAL");
    assert_eq!(pre.l1_mod, L1Modulation::Qam16, "L1_MOD");
    assert_eq!(pre.l1_post_size, 376, "L1_POST_SIZE");
    assert_eq!(pre.l1_post_info_size, 318, "L1_POST_INFO_SIZE");
    assert_eq!(pre.pilot_pattern, PilotPattern::Pp3, "PILOT_PATTERN");
    assert_eq!(pre.network_id, 0x3003, "NETWORK_ID");
    assert_eq!(pre.t2_system_id, 0x3003, "T2_SYSTEM_ID");
    assert_eq!(pre.num_t2_frames, 2, "NUM_T2_FRAMES");
    assert_eq!(pre.num_data_symbols, 41, "NUM_DATA_SYMBOLS");
    assert_eq!(pre.num_rf, 1, "NUM_RF");
    assert_eq!(pre.current_rf_idx, 0, "CURRENT_RF_IDX");
    assert_eq!(pre.t2_version, T2Version::V1_3_1, "T2_VERSION");
    assert!(!pre.l1_post_extension, "L1_POST_EXTENSION must be false");

    // ── L1Post fields ─────────────────────────────────────────────────────────
    let post = payload.l1_post().expect("l1_post() must succeed");

    // configurable
    let conf = &post.configurable;
    assert_eq!(conf.num_plp, 1, "NUM_PLP");
    assert_eq!(conf.num_aux, 0, "NUM_AUX");
    assert_eq!(conf.rf.len(), 1, "RF loop length");
    assert!(conf.fef.is_none(), "FEF block absent (S2 LSB=0)");
    assert_eq!(conf.plps.len(), 1, "PLP loop length");
    assert!(post.extension.is_empty(), "extension blocks absent");

    // PLP sanity — the Colombia fixture has one data PLP of type DataType1 (TS payload)
    let plp = &conf.plps[0];
    assert_eq!(plp.plp_type, PlpType::DataType1, "PLP_TYPE");
    assert_eq!(plp.plp_payload_type, PlpPayloadType::Ts, "PLP_PAYLOAD_TYPE");

    // Framing byte-offset accounting:
    // 21 (L1PRE) + 2 (L1CONF_LEN) + ceil(191/8)=24 (L1CONF)
    //           + 2 (L1DYN_CURR_LEN) + ceil(127/8)=16 (L1DYN_CURR)
    //           + 2 (L1EXT_LEN) + 0 (L1EXT)
    // = 21 + 2 + 24 + 2 + 16 + 2 = 67
    let conf_bits = conf.len_bits();
    assert_eq!(conf_bits, 191, "L1CONF declared bit length must be 191");
    let dyn_bits = post.dynamic_current.len_bits();
    assert_eq!(dyn_bits, 127, "L1DYN_CURR declared bit length must be 127");

    // Verify the framing adds up to exactly 67 bytes
    let total = 21 // L1PRE
        + 2 + conf_bits.div_ceil(8)   // L1CONF_LEN + L1CONF
        + 2 + dyn_bits.div_ceil(8)    // L1DYN_CURR_LEN + L1DYN_CURR
        + 2; // L1EXT_LEN + 0 bytes L1EXT
    assert_eq!(
        total,
        l1_data.len(),
        "framing byte totals must equal l1_current_data length"
    );

    // ── BYTE-EXACT ROUND-TRIP (the strongest oracle) ───────────────────────────
    // Reserialize L1PRE (21 bytes) + the framed L1-post and require it to
    // reproduce the original 67 wire bytes exactly.
    let mut reser = pre.to_bytes().to_vec();
    reser.extend_from_slice(&post.to_l1_current_framed().expect("framed serialize"));
    assert_eq!(
        reser, l1_data,
        "parse→serialize of the real L1-current data must be byte-identical"
    );
}
