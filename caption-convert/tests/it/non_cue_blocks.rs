use caption_convert::{parse_webvtt, Cue};

fn cues(input: &str) -> Vec<Cue> {
    parse_webvtt(input).unwrap().cues
}

#[test]
fn a_note_block_is_skipped_like_no_block() {
    let with_block = "WEBVTT\n\nNOTE a comment\n\n00:00:00.000 --> 00:00:01.000\ncue one\n";
    let without = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\ncue one\n";
    assert_eq!(cues(with_block), cues(without));
}

#[test]
fn a_style_block_is_skipped_like_no_block() {
    let with_block = "WEBVTT\n\nSTYLE\n::cue { color: red; }\n\n00:00:00.000 --> 00:00:01.000\ncue one\n";
    let without = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\ncue one\n";
    assert_eq!(cues(with_block), cues(without));
}

#[test]
fn a_region_block_is_skipped_like_no_block() {
    let with_block = "WEBVTT\n\nREGION\nid:r1\nwidth:40%\n\n00:00:00.000 --> 00:00:01.000\ncue one\n";
    let without = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\ncue one\n";
    assert_eq!(cues(with_block), cues(without));
}
