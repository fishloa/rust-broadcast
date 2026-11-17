//! `PesAssembler` accessor state and `StreamId::is_audio` classification.

use mpeg_pes::{PesAssembler, StreamId};

#[test]
fn assembler_state_follows_the_in_progress_unit() {
    let mut assembler = PesAssembler::new();
    assert!(assembler.is_empty());
    assert_eq!(assembler.len(), 0);
    assert!(!assembler.is_started());

    assert_eq!(assembler.feed(true, &[0xAA; 10]), None);
    assert!(!assembler.is_empty());
    assert_eq!(assembler.len(), 10);
    assert!(assembler.is_started());

    let unit = assembler.flush().expect("flushed unit");
    assert_eq!(unit, vec![0xAA; 10]);
    assert!(assembler.is_empty());
    assert_eq!(assembler.len(), 0);
    assert!(!assembler.is_started());
}

#[test]
fn only_the_audio_stream_id_range_is_audio() {
    assert!(StreamId::new(0xC0).is_audio());
    assert!(StreamId::new(0xDF).is_audio());
    assert!(!StreamId::new(0xBF).is_audio());
    assert!(!StreamId::new(0xBD).is_audio());
    assert!(!StreamId::new(0xE0).is_audio());
    assert!(!StreamId::new(0x00).is_audio());
    assert!(!StreamId::new(0xFF).is_audio());
}
