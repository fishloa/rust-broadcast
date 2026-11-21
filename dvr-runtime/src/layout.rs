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

/// The advisory lock file one recorder holds on the archive directory. It is
/// not an archive file: [`classify`] never matches it.
pub const LOCK_FILE_NAME: &str = ".lock";

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

/// A period number exactly as the writer names it: decimal digits, no sign,
/// and no leading zero except `0` itself.
fn period_digits(digits: &str) -> Option<PeriodNum> {
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    digits.parse::<u32>().ok().map(PeriodNum::new)
}

/// Classify one directory entry's name, if it is an archive file for
/// `container`. Anything else (including another container's media files, a
/// `p+1.ts`, a zero-padded `p01.m3u8`, or the transient `.pN.m3u8.tmp`) is
/// `None`.
#[must_use]
pub fn classify(name: &str, container: ArchiveContainer) -> Option<ArchiveFile> {
    let rest = name.strip_prefix('p')?;
    if let Some(digits) = rest.strip_suffix(".m3u8") {
        return period_digits(digits).map(ArchiveFile::Playlist);
    }
    let digits = rest.strip_suffix(container.ext())?;
    period_digits(digits).map(ArchiveFile::Media)
}

/// The period a transient `.pN.m3u8.tmp` belongs to, if `name` is one. It
/// never holds a committed record, so it is not an [`ArchiveFile`].
#[must_use]
pub fn classify_playlist_tmp(name: &str) -> Option<PeriodNum> {
    let digits = name.strip_prefix(".p")?.strip_suffix(".m3u8.tmp")?;
    period_digits(digits)
}

/// One file an earlier run left on disk, with the size the filesystem
/// reported. [`crate::ArchiveWriter::seed`] needs the size so byte retention
/// covers periods it did not write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeedFile {
    /// Which file this is.
    pub file: ArchiveFile,
    /// Its length in bytes.
    pub bytes: u64,
}

impl SeedFile {
    /// A seeded file and its size.
    #[must_use]
    pub fn new(file: ArchiveFile, bytes: u64) -> Self {
        Self { file, bytes }
    }
}
