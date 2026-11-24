//! Stream an MP4 file's container checks through
//! [`Mp4Demux`].
//!
//! The caller owns the open file; this module reads exactly the byte ranges the
//! stage asks for, one bounded buffer at a time, so the file is never held
//! whole. The `moov` box (a bounded unit) is kept so the `hvc1`/`hev1`
//! sample-entry warnings can be checked.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use broadcast_common::{Stage, Timestamp};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};

use crate::container_codec::ContainerCodecChecker;
use crate::report::Report;

/// Run the container checks over `file` (of `len` bytes), appending findings.
pub fn check_mp4_file(file: &mut File, len: u64, report: &mut Report) -> io::Result<()> {
    let config = Mp4DemuxConfig::default().with_total_len(Some(len));
    let mut demux = Mp4Demux::seekable(config);
    let mut checker = ContainerCodecChecker::new();
    let mut moov: Option<Vec<u8>> = None;
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let bytes = read_range(file, offset, demand.want_bytes)?;
        if moov.is_none() && is_moov_box(&bytes) {
            moov = Some(bytes.clone());
        }
        if demux
            .feed(
                Mp4Input::At {
                    offset,
                    bytes: bytes.into(),
                },
                Timestamp::ZERO,
            )
            .is_err()
        {
            break;
        }
        while let Some(event) = demux.poll() {
            checker.event(&event, report);
        }
    }
    let _ = demux.finish();
    while let Some(event) = demux.poll() {
        checker.event(&event, report);
    }
    if let Some(moov) = moov {
        checker.sample_entries(&moov, report);
    }
    Ok(())
}

fn is_moov_box(bytes: &[u8]) -> bool {
    bytes.len() >= isobmff::box_types::BOX_HEADER_MIN_SIZE
        && bytes[4..8] == isobmff::init_segment::MOOV[..]
}

/// Read exactly `len` bytes at `offset`, fewer only at end of file.
fn read_range(file: &mut File, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0u8; len];
    let filled = read_full(file, &mut buffer)?;
    buffer.truncate(filled);
    Ok(buffer)
}

fn read_full(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}
