use media_doctor::Report;
use test_fixtures::fuzz::MAX_INPUT;

pub fn check_text(data: &[u8], check: fn(&str, &mut Report)) {
    if data.len() > MAX_INPUT {
        return;
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let mut report = Report::new();
        check(s, &mut report);
        let _ = report.len();
        let _ = report.findings();
    }
}
