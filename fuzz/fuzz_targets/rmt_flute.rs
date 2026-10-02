#![no_main]

use libfuzzer_sys::fuzz_target;
use rmt_flute::{AlcPacket, LctHeader, NormCmd, NormData, NormFeedback, NormInfo};

/// Round-trip assert for a value `$v` parsed from fuzz input. Reserved bits
/// (RFC 5651 `Res`, which a receiver MUST ignore and a sender MUST zero) are
/// normalized on serialize, so equality with the *input* is not the invariant;
/// the invariant is that serialization is canonical: it fills exactly
/// `serialized_len()` bytes, re-parses to an equal value, and re-serializes to
/// the same bytes.
macro_rules! assert_canonical {
    ($what:expr, $v:expr, $b:ident => $reparse:expr) => {{
        let v = $v;
        let len = v.serialized_len();
        let mut out = vec![0u8; len];
        let n = v
            .serialize_into(&mut out)
            .unwrap_or_else(|e| panic!("{}: serialize of parsed value failed: {e}", $what));
        assert_eq!(n, len, "{}: serialized_len disagrees with bytes written", $what);
        let $b: &[u8] = &out;
        let again = ($reparse).unwrap_or_else(|e| panic!("{}: re-parse failed: {e}", $what));
        assert_eq!(again, v, "{}: re-parse differs", $what);
        let mut out2 = vec![0u8; len];
        again.serialize_into(&mut out2).unwrap();
        assert_eq!(out, out2, "{}: serialization not canonical", $what);
    }};
}

fuzz_target!(|data: &[u8]| {
    // RFC 5651 LCT header.
    if let Ok((lct, used)) = LctHeader::parse(data) {
        assert_eq!(lct.serialized_len(), used, "LctHeader length mismatch");
        assert_canonical!(
            "LctHeader",
            lct,
            b => LctHeader::parse(b).map(|(h, _)| h)
        );
    }

    // The FEC Payload ID length is scheme-defined; derive it from the input
    // so every small size is exercised.
    let fec_len = data.first().map_or(0, |b| usize::from(b & 0x0F));

    // RFC 5775 ALC packet (LCT + FEC Payload ID + payload).
    if let Ok(pkt) = AlcPacket::parse(data, fec_len) {
        assert_canonical!(
            "AlcPacket",
            pkt,
            b => AlcPacket::parse(b, fec_len)
        );
    }

    // RFC 5740 NORM messages.
    if let Ok(m) = NormInfo::parse(data) {
        assert_canonical!("NormInfo", m, b => NormInfo::parse(b));
    }
    if let Ok(m) = NormData::parse(data, fec_len) {
        assert_canonical!("NormData", m, b => NormData::parse(b, fec_len));
    }
    if let Ok(m) = NormCmd::parse(data, fec_len) {
        assert_canonical!("NormCmd", m, b => NormCmd::parse(b, fec_len));
    }
    if let Ok(m) = NormFeedback::parse(data) {
        assert_canonical!(
            "NormFeedback",
            m,
            b => NormFeedback::parse(b)
        );
    }
});
