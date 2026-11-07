use caption_convert::{Error, parse_webvtt, webvtt_to_srt};

#[test]
fn an_empty_document_is_empty_input() {
    assert_eq!(parse_webvtt("").unwrap_err(), Error::EmptyInput);
}

#[test]
fn a_malformed_cue_timestamp_is_an_invalid_timestamp_naming_it() {
    let doc = "WEBVTT\n\n00:00:xx.000 --> 00:00:02.000\nhello\n";
    assert_eq!(
        webvtt_to_srt(doc).unwrap_err(),
        Error::InvalidTimestamp("00:00:xx.000".to_string())
    );
}
