//! Configuration and error-surface tests (#3666, #3667, #3672).

use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;

use broadcast_common::range::ByteSpan;
use dvr_runtime::read::ReadError;
use dvr_runtime::{
    ArchiveConfig, ArchiveContainer, ArchiveName, ConfigError, MAX_ARCHIVE_NAME_LEN,
    RetentionLimits,
};

fn retention() -> RetentionLimits {
    RetentionLimits::new().with_periods(NonZeroUsize::new(2).expect("nonzero"))
}

#[test]
fn archive_name_length_boundary_is_exact() {
    let max = "a".repeat(MAX_ARCHIVE_NAME_LEN);
    assert_eq!(
        ArchiveName::new(&max)
            .expect("255 bytes is accepted")
            .as_str()
            .len(),
        255
    );
    let over = "a".repeat(MAX_ARCHIVE_NAME_LEN + 1);
    assert!(matches!(
        ArchiveName::new(&over),
        Err(ConfigError::NameTooLong { len: 256, max: 255 })
    ));
}

#[test]
fn archive_name_rejects_unsafe_components() {
    assert!(matches!(ArchiveName::new(""), Err(ConfigError::EmptyName)));
    assert!(matches!(
        ArchiveName::new("."),
        Err(ConfigError::TraversalName)
    ));
    assert!(matches!(
        ArchiveName::new(".."),
        Err(ConfigError::TraversalName)
    ));
    assert!(matches!(
        ArchiveName::new("a..b"),
        Err(ConfigError::TraversalName)
    ));
    for bad in ["a/b", "a\\b", "a\u{0}b"] {
        assert!(
            matches!(ArchiveName::new(bad), Err(ConfigError::SeparatorInName)),
            "{bad:?} must be rejected"
        );
    }
    assert_eq!(ArchiveName::new("cam-1").expect("safe").as_str(), "cam-1");
}

#[test]
fn config_validation_names_each_rule() {
    let no_retention = ArchiveConfig::new(
        ArchiveContainer::Ts,
        Duration::from_secs(2),
        RetentionLimits::new(),
    );
    assert!(matches!(
        no_retention.validate(),
        Err(ConfigError::NoRetention)
    ));

    let unbounded = ArchiveConfig::new(ArchiveContainer::Ts, Duration::from_secs(2), retention());
    assert!(matches!(
        unbounded.validate(),
        Err(ConfigError::UnboundedPeriod)
    ));

    let zero_target = ArchiveConfig::new(ArchiveContainer::Ts, Duration::ZERO, retention())
        .with_period_duration(Some(Duration::from_secs(10)));
    assert!(matches!(
        zero_target.validate(),
        Err(ConfigError::ZeroTargetDuration)
    ));

    let mut ok = ArchiveConfig::new(ArchiveContainer::Fmp4, Duration::from_secs(2), retention());
    assert!(matches!(ok.validate(), Err(ConfigError::UnboundedPeriod)));
    ok = ok.with_period_duration(Some(Duration::from_secs(10)));
    ok.validate().expect("a time roll bounds the period");

    let byte_bounded = ArchiveConfig::new(
        ArchiveContainer::Fmp4,
        Duration::from_secs(2),
        RetentionLimits::new().with_bytes(NonZeroU64::new(1).expect("nonzero")),
    );
    byte_bounded
        .validate()
        .expect("a byte cap bounds the period");
}

#[test]
fn retention_limits_and_budget_are_exact() {
    assert!(!RetentionLimits::new().is_active());
    assert!(retention().is_active());
    let budget = RetentionLimits::new()
        .with_periods(NonZeroUsize::new(3).expect("nonzero"))
        .with_bytes(NonZeroU64::new(3000).expect("nonzero"))
        .per_period_byte_budget()
        .expect("budget")
        .get();
    assert_eq!(budget, 750);
    let halves = RetentionLimits::new()
        .with_bytes(NonZeroU64::new(9).expect("nonzero"))
        .per_period_byte_budget()
        .expect("budget")
        .get();
    assert_eq!(
        halves, 9,
        "9 / (0 + 1) = 9: only the open period is bounded"
    );
}

#[test]
fn read_error_display_names_the_failure() {
    let gone = ReadError::Gone {
        period: dvr_runtime::PeriodNum::new(7),
    };
    assert!(gone.to_string().contains("period 7"), "{gone}");
    assert!(gone.to_string().contains("evicted"), "{gone}");
    let span = ByteSpan::new(10, NonZeroU64::new(5).expect("nonzero"));
    let beyond = ReadError::RangeBeyondFile { span, file_len: 12 };
    assert!(beyond.to_string().contains("10-14"), "{beyond}");
    assert!(beyond.to_string().contains("12"), "{beyond}");
}

#[test]
fn the_layout_names_every_file_and_classifies_it_back() {
    use dvr_runtime::layout::{ArchiveFile, classify, file_name};
    for (container, ext) in [
        (ArchiveContainer::Fmp4, "m4s"),
        (ArchiveContainer::Ts, "ts"),
    ] {
        let media = ArchiveFile::Media(dvr_runtime::PeriodNum::new(7));
        assert_eq!(file_name(media, container), format!("p7.{ext}"));
        assert_eq!(classify(&format!("p7.{ext}"), container), Some(media));
        let playlist = ArchiveFile::Playlist(dvr_runtime::PeriodNum::new(7));
        assert_eq!(file_name(playlist, container), "p7.m3u8");
        assert_eq!(classify("p7.m3u8", container), Some(playlist));
        assert_eq!(classify("p7.m3u8.tmp", container), None);
        assert_eq!(classify(".p7.m3u8.tmp", container), None);
        assert_eq!(classify("p7.event.json", container), None);
    }
    assert_eq!(classify("p7.m4s", ArchiveContainer::Ts), None);
    assert_eq!(classify("p7.ts", ArchiveContainer::Fmp4), None);
    let last = dvr_runtime::PeriodNum::new(u32::MAX);
    assert_eq!(last.checked_next(), None);
    assert_eq!(
        dvr_runtime::PeriodNum::new(1).checked_next(),
        Some(dvr_runtime::PeriodNum::new(2))
    );
}
