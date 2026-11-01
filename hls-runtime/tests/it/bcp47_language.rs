#![cfg(feature = "std")]

use hls_runtime::bcp47_language;

#[test]
fn canonicalises() {
    assert_eq!(bcp47_language("fre"), Some("fr".to_string()));
    assert_eq!(bcp47_language("fra"), Some("fr".to_string()));
    assert_eq!(bcp47_language("ger"), Some("de".to_string()));
    assert_eq!(bcp47_language("deu"), Some("de".to_string()));
    assert_eq!(bcp47_language("eng"), Some("en".to_string()));
    assert_eq!(bcp47_language("dut"), Some("nl".to_string()));
    assert_eq!(bcp47_language("chi"), Some("zh".to_string()));
    assert_eq!(bcp47_language("qaa"), Some("qaa".to_string()));
    assert_eq!(bcp47_language("qad"), Some("qad".to_string()));
    assert_eq!(bcp47_language("und"), Some("und".to_string()));
    assert_eq!(bcp47_language("FRE"), Some("fr".to_string()));
    assert_eq!(bcp47_language("xyz"), None);
}
