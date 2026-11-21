//! The archive's file layout: one directory per archive, one media file and
//! one playlist per period.
//!
//! ```text
//! <archive_root>/<archive_name>/
//!   pN.m4s | pN.ts   period media file
//!   pN.m3u8          period playlist (the index)
//!   .pN.m3u8.tmp     transient during playlist creation
//! ```

use std::path::{Path, PathBuf};

use crate::{ArchiveContainer, ArchiveName, PeriodNum};

/// One file the archive holds. Period numbers are decimal and unpadded, so a
/// name classifies back to exactly one kind and period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ArchiveFile {
    /// `pN.m4s` / `pN.ts` — the period's media file.
    Media(PeriodNum),
    /// `pN.m3u8` — the period's playlist.
    Playlist(PeriodNum),
}

impl ArchiveFile {
    /// The period this file belongs to.
    #[must_use]
    pub fn period(&self) -> PeriodNum {
        match self {
            Self::Media(period) | Self::Playlist(period) => *period,
        }
    }
}

/// The directory one archive lives in: `<root>/<name>/`.
#[must_use]
pub fn route_dir(root: &Path, name: &ArchiveName) -> PathBuf {
    root.join(name.as_str())
}

/// The period's media file name (`pN.m4s` / `pN.ts`).
#[must_use]
pub fn media_file_name(period: PeriodNum, container: ArchiveContainer) -> String {
    format!("p{}{}", period.value(), container.ext())
}

/// The period's playlist name (`pN.m3u8`).
#[must_use]
pub fn playlist_file_name(period: PeriodNum) -> String {
    format!("p{}.m3u8", period.value())
}

/// The temporary file a playlist is written through.
#[must_use]
pub fn playlist_tmp_name(period: PeriodNum) -> String {
    format!(".p{}.m3u8.tmp", period.value())
}

/// The file name of `file` for `container`.
#[must_use]
pub fn file_name(file: ArchiveFile, container: ArchiveContainer) -> String {
    match file {
        ArchiveFile::Media(period) => media_file_name(period, container),
        ArchiveFile::Playlist(period) => playlist_file_name(period),
    }
}

/// Classify one directory entry's name, if it is an archive file for
/// `container`. Anything else (including another container's media files) is
/// `None`.
#[must_use]
pub fn classify(name: &str, container: ArchiveContainer) -> Option<ArchiveFile> {
    let rest = name.strip_prefix('p')?;
    if let Some(digits) = rest.strip_suffix(".m3u8") {
        return digits
            .parse::<u32>()
            .ok()
            .map(|n| ArchiveFile::Playlist(PeriodNum::new(n)));
    }
    let digits = rest.strip_suffix(container.ext())?;
    digits
        .parse::<u32>()
        .ok()
        .map(|n| ArchiveFile::Media(PeriodNum::new(n)))
}
