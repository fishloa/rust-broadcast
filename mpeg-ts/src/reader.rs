//! Streaming TS packet framing over any [`std::io::Read`].
//!
//! [`PacketReader`] turns a byte stream into its 188-byte transport-stream
//! packets, resynchronising through [`TsResync`] as
//! the stream arrives, so a multi-gigabyte capture is framed in bounded
//! memory instead of being read whole.
//!
//! ```no_run
//! use std::fs::File;
//! use mpeg_ts::reader::PacketReader;
//!
//! let mut packets = PacketReader::new(File::open("capture.ts")?);
//! while let Some(packet) = packets.next() {
//!     let packet = packet?;
//!     // one aligned 188-byte TS packet
//!     let _ = packet.len();
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::io::{self, Read};

use alloc::vec::Vec;

use crate::resync::TsResync;
use crate::ts::TS_PACKET_SIZE;

/// Bytes one read asks the source for: large enough to amortise the syscall,
/// small enough that memory stays flat on a multi-gigabyte input.
pub const READ_CHUNK_LEN: usize = 1 << 20;

/// The aligned 188-byte packets of a byte stream, read in bounded chunks.
///
/// A capture that begins mid-packet, carries a corrupted sync byte, or is
/// 204-byte RS-coded is resynchronised rather than rejected. A read error ends
/// the iteration and is returned once; a short trailing packet is dropped, as
/// [`TsResync::finish`](crate::resync::TsResync::finish) reports it as a loss.
pub struct PacketReader<R> {
    reader: R,
    resync: TsResync,
    chunk: Vec<u8>,
    pending: alloc::vec::IntoIter<[u8; TS_PACKET_SIZE]>,
    done: bool,
}

impl<R: Read> PacketReader<R> {
    /// Frame the packets of `reader`, reading [`READ_CHUNK_LEN`] bytes at a
    /// time.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            resync: TsResync::new(),
            chunk: alloc::vec![0u8; READ_CHUNK_LEN],
            pending: Vec::new().into_iter(),
            done: false,
        }
    }

    /// One `read`, retried on [`io::ErrorKind::Interrupted`].
    fn read_chunk(&mut self) -> io::Result<usize> {
        loop {
            match self.reader.read(&mut self.chunk) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                other => return other,
            }
        }
    }
}

impl<R: Read> Iterator for PacketReader<R> {
    type Item = io::Result<[u8; TS_PACKET_SIZE]>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(packet) = self.pending.next() {
                return Some(Ok(packet));
            }
            if self.done {
                return None;
            }
            match self.read_chunk() {
                Ok(0) => {
                    self.resync.finish();
                    self.pending = self.resync.take_tail_packets().into_iter();
                    self.done = true;
                }
                Ok(read) => {
                    self.pending = self.resync.feed(&self.chunk[..read]).into_iter();
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source that hands out its bytes `step` at a time, so a packet
    /// boundary can fall anywhere.
    struct Chunked {
        data: Vec<u8>,
        offset: usize,
        step: usize,
    }

    impl Read for Chunked {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let remaining = self.data.len() - self.offset;
            let count = remaining.min(buf.len()).min(self.step);
            buf[..count].copy_from_slice(&self.data[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }
    }

    fn packet(fill: u8) -> [u8; TS_PACKET_SIZE] {
        let mut packet = [fill; TS_PACKET_SIZE];
        packet[0] = crate::ts::TS_SYNC_BYTE;
        packet
    }

    /// Every packet is yielded whatever the read boundaries are, and a
    /// trailing partial packet is dropped.
    #[test]
    fn packets_survive_any_chunk_boundary() {
        let mut data = Vec::new();
        for fill in 0u8..8 {
            data.extend_from_slice(&packet(fill));
        }
        data.extend_from_slice(&[0x47, 0x00, 0x00]);
        let expected: Vec<[u8; TS_PACKET_SIZE]> = (0u8..8).map(packet).collect();

        for step in [1usize, 7, 188, 500] {
            let reader = Chunked {
                data: data.clone(),
                offset: 0,
                step,
            };
            let mut seen = Vec::new();
            for result in PacketReader::new(reader) {
                seen.push(result.expect("an in-memory source cannot fail"));
            }
            assert_eq!(seen, expected, "step {step}");
        }
    }

    /// A capture that begins mid-packet still yields whole packets.
    #[test]
    fn a_mid_packet_start_is_resynchronised() {
        let mut data = alloc::vec![0x00u8; 7];
        for fill in 1u8..8 {
            data.extend_from_slice(&packet(fill));
        }
        let reader = Chunked {
            data,
            offset: 0,
            step: 64,
        };
        let mut seen = Vec::new();
        for result in PacketReader::new(reader) {
            seen.push(result.expect("an in-memory source cannot fail"));
        }
        let expected: Vec<[u8; TS_PACKET_SIZE]> = (1u8..8).map(packet).collect();
        assert_eq!(seen, expected);
    }
}
