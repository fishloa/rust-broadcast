//! ISO 639 -> BCP 47 (RFC 5646) language canonicalisation.
//!
//! A single home for the mapping shared by the HLS and DASH paths: an ISO
//! 639-2 (bibliographic or terminology) code is canonicalised to the BCP 47
//! primary language subtag (ISO 639-1 two-letter code where one exists, else
//! the ISO 639-2/T three-letter code). Backed by the `iso-639` crate; no
//! hand-rolled code table.

/// Canonicalise an ISO 639-2 language code into a BCP 47 (RFC 5646) primary
/// language subtag.
///
/// HLS `LANGUAGE` and DASH `AdaptationSet@lang` must be RFC 5646 language tags;
/// in BCP 47 the primary subtag is the ISO 639-1 two-letter code when one
/// exists, otherwise the ISO 639-2/T (== ISO 639-3) three-letter code. ISO
/// 639-2/B bibliographic codes (`fre`, `ger`, `dut`, `chi`, ...) are not valid
/// BCP 47 subtags and are mapped to their terminology (T) equivalents first.
///
/// `qaa`..`qtz` (private use), `und`, `mul`, `mis` and `zxx` pass through
/// unchanged. Unknown or malformed codes return `None`.
pub fn bcp47_language(iso639_2: &str) -> Option<String> {
    let code = iso639_2.trim().to_ascii_lowercase();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    // Private-use range and the reserved singleton codes pass through verbatim.
    if code == "und" || code == "mul" || code == "mis" || code == "zxx" {
        return Some(code);
    }
    if code.starts_with('q') {
        let bytes = code.as_bytes();
        if (b'a'..=b't').contains(&bytes[1]) && bytes[2].is_ascii_lowercase() {
            return Some(code);
        }
    }
    let lang = iso_639::Language::from_str(&code)?;
    if let Some(a2) = lang.alpha2() {
        return Some(a2.to_string());
    }
    if let Some(t) = lang.alpha3_t() {
        return Some(t.to_string());
    }
    lang.alpha3_b().map(|b| b.to_string())
}
