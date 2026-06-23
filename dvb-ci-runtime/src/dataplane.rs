//! The CI **TS data-plane** device: [`CiDataDevice`].
//!
//! Separate-CI hardware (CXD2099-class CI bridges — a PCIe/USB CI card with no
//! integrated demod) exposes two devices. The **control plane** is the CA device
//! ([`CaDevice`](crate::device::CaDevice), `caM`): the EN 50221 APDU exchange
//! (resource manager, sessions, `ca_pmt`). The **data plane** is `ciM`: the host
//! **writes** the scrambled Transport Stream into it and **reads** the
//! descrambled TS back out. This module abstracts the data plane the same way
//! [`CaDevice`](crate::device::CaDevice) abstracts the control plane, so the
//! host-fed descramble path can be driven by a real device (`linux` feature) or
//! an in-memory mock.
//!
//! All TS I/O is in whole 188-byte packets ([`TS_PACKET_LEN`]); a non-aligned
//! buffer is rejected with [`io::ErrorKind::InvalidInput`].

use std::io;
use std::time::Duration;

/// MPEG-2 TS packet length (ISO/IEC 13818-1): 188 bytes.
pub const TS_PACKET_LEN: usize = 188;

/// The TS data-plane device of a separate-CI module (`/dev/dvb/adapterN/ciM`).
///
/// The host pushes scrambled TS in with [`write`](Self::write) and pulls the
/// descrambled TS out with [`read`](Self::read). Implementations:
/// [`MockCiDataDevice`] (in-memory, for tests + differential harness) and, with
/// the `linux` feature, a device over `/dev/dvb/.../ci`.
pub trait CiDataDevice {
    /// Write scrambled TS to the module. `ts` must be a whole number of
    /// [`TS_PACKET_LEN`]-byte packets.
    fn write(&mut self, ts: &[u8]) -> io::Result<()>;

    /// Read descrambled TS into `buf` (sized to a multiple of [`TS_PACKET_LEN`]);
    /// returns the byte count. `Ok(0)` means none available (non-blocking).
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// Wait up to `timeout` for descrambled TS to become readable; `Ok(true)` if
    /// readable.
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
}

/// Reject a buffer that is not a whole number of TS packets.
fn check_aligned(len: usize, what: &'static str) -> io::Result<()> {
    if len % TS_PACKET_LEN == 0 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, what))
    }
}

/// In-memory [`CiDataDevice`] for tests and the differential harness.
///
/// - `descrambled` is a scripted queue of TS the "module" returns; each
///   [`read`](CiDataDevice::read) pops one entry (the "scripted descramble").
/// - every byte the host writes is recorded in `written`, so a test (or a
///   byte-exact comparison against an external reference) can assert the exact
///   scrambled TS that was pushed in.
#[derive(Debug, Default)]
pub struct MockCiDataDevice {
    /// Scripted descrambled-TS the module returns to the host (FIFO).
    pub descrambled: std::collections::VecDeque<Vec<u8>>,
    /// Scrambled TS the host wrote, in order.
    pub written: Vec<Vec<u8>>,
}

impl MockCiDataDevice {
    /// New mock returning the given descrambled-TS script.
    #[must_use]
    pub fn new(descrambled: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            descrambled: descrambled.into_iter().collect(),
            written: Vec::new(),
        }
    }

    /// All scrambled TS the host wrote, concatenated.
    #[must_use]
    pub fn written_ts(&self) -> Vec<u8> {
        self.written.iter().flatten().copied().collect()
    }
}

impl CiDataDevice for MockCiDataDevice {
    fn write(&mut self, ts: &[u8]) -> io::Result<()> {
        check_aligned(ts.len(), "write not a multiple of 188 bytes")?;
        self.written.push(ts.to_vec());
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        check_aligned(buf.len(), "read buffer not a multiple of 188 bytes")?;
        match self.descrambled.pop_front() {
            Some(ts) => {
                let n = ts.len().min(buf.len());
                buf[..n].copy_from_slice(&ts[..n]);
                Ok(n)
            }
            None => Ok(0),
        }
    }

    fn poll(&mut self, _timeout: Duration) -> io::Result<bool> {
        Ok(!self.descrambled.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(fill: u8) -> Vec<u8> {
        let mut p = vec![fill; TS_PACKET_LEN];
        p[0] = 0x47; // sync byte
        p
    }

    #[test]
    fn mock_records_writes_and_replays_descrambled() {
        let mut dev = MockCiDataDevice::new([packet(0xAA), packet(0xBB)]);
        dev.write(&packet(0x11)).unwrap();
        dev.write(&packet(0x22)).unwrap();

        let mut buf = [0u8; TS_PACKET_LEN];
        assert!(dev.poll(Duration::ZERO).unwrap());
        assert_eq!(dev.read(&mut buf).unwrap(), TS_PACKET_LEN);
        assert_eq!(buf[1], 0xAA);
        assert_eq!(dev.read(&mut buf).unwrap(), TS_PACKET_LEN);
        assert_eq!(buf[1], 0xBB);
        assert_eq!(dev.read(&mut buf).unwrap(), 0); // drained
        assert!(!dev.poll(Duration::ZERO).unwrap());

        assert_eq!(dev.written_ts().len(), 2 * TS_PACKET_LEN);
    }

    #[test]
    fn rejects_unaligned_io() {
        let mut dev = MockCiDataDevice::new([]);
        assert_eq!(
            dev.write(&[0x47; 100]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let mut buf = [0u8; 100];
        assert_eq!(
            dev.read(&mut buf).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
