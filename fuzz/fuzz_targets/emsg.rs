#![no_main]

use libfuzzer_sys::fuzz_target;
use mp4_emsg::EmsgBox;

fuzz_target!(|data: &[u8]| {
    let Ok(parsed) = EmsgBox::parse(data) else {
        return;
    };
    let bytes = parsed.to_vec().expect("a parsed emsg serializes");
    let reparsed = EmsgBox::parse(&bytes).expect("serialized emsg reparses");
    assert_eq!(parsed, reparsed, "emsg parse->serialize->parse");
});
