use caption_convert::parse_webvtt;

#[test]
fn cue_identifier_starting_with_note_is_not_a_note_block() {
    let doc = "WEBVTT\n\nNOTES1\n00:00:01.000 --> 00:00:02.000\nhello\n";
    let parsed = parse_webvtt(doc).unwrap();
    assert_eq!(parsed.cues.len(), 1);
    assert_eq!(parsed.cues[0].text, "hello");
}
